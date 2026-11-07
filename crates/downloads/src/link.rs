//! Which links the engine offers to save, and their canonical form.
//!
//! Hand-rolled on purpose: the accepted shapes are few and fixed, and a full URL parser would
//! not tell us which paths are media anyway.

use crate::format::MediaKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Site {
    YouTube,
    YouTubeMusic,
    Instagram,
    TikTok,
    X,
    SoundCloud,
}

impl Site {
    pub const ALL: [Site; 6] = [Site::YouTube, Site::YouTubeMusic, Site::Instagram, Site::TikTok, Site::X, Site::SoundCloud];

    /// Plain text name for the UI (the design rules forbid site logos).
    pub fn name(self) -> &'static str {
        match self {
            Site::YouTube => "YouTube",
            Site::YouTubeMusic => "YouTube Music",
            Site::Instagram => "Instagram",
            Site::TikTok => "TikTok",
            Site::X => "X",
            Site::SoundCloud => "SoundCloud",
        }
    }

    /// Music sites always start as audio.
    pub fn default_kind(self) -> MediaKind {
        match self {
            Site::YouTubeMusic | Site::SoundCloud => MediaKind::Audio,
            _ => MediaKind::Video,
        }
    }
}

/// A supported media link in canonical form. Only [`MediaLink::parse`] builds one, so holding a
/// `MediaLink` means the site is supported and the URL is normalised.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MediaLink {
    site: Site,
    url: String,
}

impl MediaLink {
    /// Accepts text that is exactly one URL (surrounding whitespace allowed, scheme optional).
    pub fn parse(text: &str) -> Option<MediaLink> {
        let p = Parts::split(text)?;
        let seg = p.segments.as_slice();
        let (site, url) = match p.host.as_str() {
            "youtube.com" => (Site::YouTube, youtube(seg, p.query)?),
            "youtu.be" => match seg {
                [id] if yt_id(id) => (Site::YouTube, format!("https://www.youtube.com/watch?v={id}")),
                _ => return None,
            },
            "music.youtube.com" => match seg {
                ["watch"] => {
                    let id = param(p.query, "v").filter(|id| yt_id(id))?;
                    (Site::YouTubeMusic, format!("https://music.youtube.com/watch?v={id}"))
                }
                _ => return None,
            },
            "instagram.com" => (Site::Instagram, instagram(seg)?),
            "tiktok.com" => (Site::TikTok, tiktok(seg)?),
            "vm.tiktok.com" | "vt.tiktok.com" => match seg {
                [code] if token(code) => (Site::TikTok, format!("https://{}/{code}/", p.host)),
                _ => return None,
            },
            "x.com" | "twitter.com" => match seg {
                [user, "status", id, ..] if token(user) && digits(id) => (Site::X, format!("https://x.com/{user}/status/{id}")),
                _ => return None,
            },
            "soundcloud.com" => (Site::SoundCloud, soundcloud(seg)?),
            "on.soundcloud.com" => match seg {
                [code] if token(code) => (Site::SoundCloud, format!("https://on.soundcloud.com/{code}")),
                _ => return None,
            },
            _ => return None,
        };
        Some(MediaLink { site, url })
    }

    pub fn site(&self) -> Site {
        self.site
    }

    /// The canonical URL handed to yt-dlp, also the key for "Already saved".
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// True when the text is a link the engine offers to save. Unsupported links are never offered.
pub fn is_media_link(text: &str) -> bool {
    MediaLink::parse(text).is_some()
}

struct Parts<'a> {
    /// Lowercase, with a leading `www.`, `m.` or `mobile.` removed.
    host: String,
    segments: Vec<&'a str>,
    query: &'a str,
}

impl<'a> Parts<'a> {
    fn split(text: &'a str) -> Option<Parts<'a>> {
        let t = text.trim();
        if t.is_empty() || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return None;
        }
        let rest = match t.find("://") {
            Some(i) => {
                let scheme = &t[..i];
                if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
                    return None;
                }
                &t[i + 3..]
            }
            None => t,
        };
        let rest = rest.split('#').next()?;
        let (before_query, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (host, path) = before_query.split_once('/').unwrap_or((before_query, ""));
        // A port or user info never appears in a shared media link.
        if host.is_empty() || host.contains(['@', ':']) {
            return None;
        }
        let mut host = host.trim_end_matches('.').to_ascii_lowercase();
        for prefix in ["www.", "m.", "mobile."] {
            if let Some(h) = host.strip_prefix(prefix) {
                host = h.to_string();
                break;
            }
        }
        Some(Parts { host, segments: path.split('/').filter(|s| !s.is_empty()).collect(), query })
    }
}

fn youtube(seg: &[&str], query: &str) -> Option<String> {
    let id = match seg {
        ["watch"] => param(query, "v")?,
        ["shorts", id] => id,
        _ => return None,
    };
    yt_id(id).then(|| format!("https://www.youtube.com/watch?v={id}"))
}

fn instagram(seg: &[&str]) -> Option<String> {
    let (kind, code) = match seg {
        ["reel" | "reels", code] | [_, "reel", code] => ("reel", *code),
        ["p", code] | [_, "p", code] => ("p", *code),
        _ => return None,
    };
    token(code).then(|| format!("https://www.instagram.com/{kind}/{code}/"))
}

fn tiktok(seg: &[&str]) -> Option<String> {
    match seg {
        [user, "video", id] if user.len() > 1 && user.starts_with('@') && token(&user[1..]) && digits(id) => {
            Some(format!("https://www.tiktok.com/{user}/video/{id}"))
        }
        ["t", code] if token(code) => Some(format!("https://www.tiktok.com/t/{code}/")),
        _ => None,
    }
}

fn soundcloud(seg: &[&str]) -> Option<String> {
    // First segments that are SoundCloud pages, not artists.
    const PAGES: [&str; 14] = [
        "discover", "stream", "search", "you", "upload", "charts", "pages", "settings", "messages", "notifications", "people",
        "tags", "jobs", "terms-of-use",
    ];
    // Second segments that are lists, not tracks. `sets` is a playlist, which we never offer.
    const LISTS: [&str; 9] = ["sets", "tracks", "albums", "likes", "reposts", "followers", "following", "popular-tracks", "comments"];
    let (artist, track, secret) = match seg {
        [a, t] => (*a, *t, None),
        [a, t, s] if s.starts_with("s-") && token(s) => (*a, *t, Some(*s)),
        _ => return None,
    };
    if !token(artist) || !token(track) || PAGES.contains(&artist) || LISTS.contains(&track) {
        return None;
    }
    Some(match secret {
        Some(s) => format!("https://soundcloud.com/{artist}/{track}/{s}"),
        None => format!("https://soundcloud.com/{artist}/{track}"),
    })
}

fn param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

fn yt_id(s: &str) -> bool {
    s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.len() <= 25 && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(text: &str) -> Option<(Site, String)> {
        MediaLink::parse(text).map(|l| (l.site(), l.url().to_string()))
    }

    fn yt(id: &str) -> Option<(Site, String)> {
        Some((Site::YouTube, format!("https://www.youtube.com/watch?v={id}")))
    }

    #[test]
    fn youtube_watch_short_link_and_shorts_share_one_canonical_url() {
        assert_eq!(norm("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), yt("dQw4w9WgXcQ"));
        assert_eq!(norm("https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&list=PL1&t=42"), yt("dQw4w9WgXcQ"));
        assert_eq!(norm("http://m.youtube.com/watch?v=dQw4w9WgXcQ#t=1"), yt("dQw4w9WgXcQ"));
        assert_eq!(norm("https://youtu.be/dQw4w9WgXcQ?si=abcDEF"), yt("dQw4w9WgXcQ"));
        assert_eq!(norm("youtu.be/dQw4w9WgXcQ"), yt("dQw4w9WgXcQ"));
        assert_eq!(norm("  https://www.youtube.com/shorts/aBcDeFgHiJk/  \n"), yt("aBcDeFgHiJk"));
        assert_eq!(norm("HTTPS://WWW.YOUTUBE.COM/watch?v=dQw4w9WgXcQ"), yt("dQw4w9WgXcQ"));
    }

    #[test]
    fn youtube_pages_that_are_not_one_video_are_not_offered() {
        for text in [
            "https://www.youtube.com/",
            "https://www.youtube.com/playlist?list=PL123",
            "https://www.youtube.com/@channel",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch?list=PL123",
            "https://youtu.be/",
            "https://www.youtube.com:8080/watch?v=dQw4w9WgXcQ",
            "ftp://youtube.com/watch?v=dQw4w9WgXcQ",
            "watch this https://youtu.be/dQw4w9WgXcQ",
        ] {
            assert!(!is_media_link(text), "{text}");
        }
    }

    #[test]
    fn youtube_music_is_its_own_site_and_starts_as_audio() {
        let l = MediaLink::parse("https://music.youtube.com/watch?v=q7Lm3kVxYz0&si=x").unwrap();
        assert_eq!(l.site(), Site::YouTubeMusic);
        assert_eq!(l.url(), "https://music.youtube.com/watch?v=q7Lm3kVxYz0");
        assert_eq!(l.site().default_kind(), MediaKind::Audio);
        assert!(!is_media_link("https://music.youtube.com/playlist?list=OLAK5uy"));
    }

    #[test]
    fn instagram_reels_and_posts() {
        assert_eq!(
            norm("https://www.instagram.com/reel/C9xYz_AbCd/?igsh=MTIz"),
            Some((Site::Instagram, "https://www.instagram.com/reel/C9xYz_AbCd/".into()))
        );
        assert_eq!(norm("https://instagram.com/reels/C9xYz_AbCd"), Some((Site::Instagram, "https://www.instagram.com/reel/C9xYz_AbCd/".into())));
        assert_eq!(norm("https://www.instagram.com/p/Cabc-123/"), Some((Site::Instagram, "https://www.instagram.com/p/Cabc-123/".into())));
        assert_eq!(norm("https://www.instagram.com/someone/reel/C9xYz/"), Some((Site::Instagram, "https://www.instagram.com/reel/C9xYz/".into())));
        assert!(!is_media_link("https://www.instagram.com/someone/"));
        assert!(!is_media_link("https://www.instagram.com/stories/someone/123/"));
    }

    #[test]
    fn tiktok_videos_and_short_links() {
        assert_eq!(
            norm("https://www.tiktok.com/@some.user/video/7301234567890123456?is_from_webapp=1"),
            Some((Site::TikTok, "https://www.tiktok.com/@some.user/video/7301234567890123456".into()))
        );
        assert_eq!(norm("https://vm.tiktok.com/ZMabc123/"), Some((Site::TikTok, "https://vm.tiktok.com/ZMabc123/".into())));
        assert_eq!(norm("https://www.tiktok.com/t/ZTabc123/"), Some((Site::TikTok, "https://www.tiktok.com/t/ZTabc123/".into())));
        assert!(!is_media_link("https://www.tiktok.com/@some.user"));
        assert!(!is_media_link("https://www.tiktok.com/@/video/123"));
    }

    #[test]
    fn x_and_twitter_statuses_become_x() {
        let want = Some((Site::X, "https://x.com/someone/status/1850000000000000000".into()));
        assert_eq!(norm("https://x.com/someone/status/1850000000000000000"), want);
        assert_eq!(norm("https://twitter.com/someone/status/1850000000000000000?s=20"), want);
        assert_eq!(norm("https://mobile.twitter.com/someone/status/1850000000000000000/video/1"), want);
        assert!(!is_media_link("https://x.com/someone"));
        assert!(!is_media_link("https://x.com/someone/status/abc"));
    }

    #[test]
    fn soundcloud_tracks_are_audio_and_sets_are_not_offered() {
        let l = MediaLink::parse("https://soundcloud.com/artist-name/track-name?si=abc&utm_source=clipboard").unwrap();
        assert_eq!(l.site(), Site::SoundCloud);
        assert_eq!(l.url(), "https://soundcloud.com/artist-name/track-name");
        assert_eq!(l.site().default_kind(), MediaKind::Audio);
        assert_eq!(norm("https://m.soundcloud.com/a/b/s-SeCrEt1"), Some((Site::SoundCloud, "https://soundcloud.com/a/b/s-SeCrEt1".into())));
        assert_eq!(norm("https://on.soundcloud.com/AbC123"), Some((Site::SoundCloud, "https://on.soundcloud.com/AbC123".into())));
        assert!(!is_media_link("https://soundcloud.com/artist-name/sets/album"));
        assert!(!is_media_link("https://soundcloud.com/discover/sets"));
        assert!(!is_media_link("https://soundcloud.com/artist-name"));
    }

    #[test]
    fn other_sites_and_non_urls_are_not_offered() {
        for text in ["", "hello", "https://vimeo.com/123456", "https://example.com/watch?v=dQw4w9WgXcQ", "https://notyoutube.com/watch?v=dQw4w9WgXcQ", "https://user@youtube.com/watch?v=dQw4w9WgXcQ"] {
            assert!(!is_media_link(text), "{text}");
        }
    }

    #[test]
    fn copying_the_same_video_twice_gives_equal_links() {
        assert_eq!(MediaLink::parse("https://youtu.be/dQw4w9WgXcQ"), MediaLink::parse("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=9"));
    }
}
