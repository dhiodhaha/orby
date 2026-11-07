//! The yt-dlp command line for one download.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::format::{AudioCodec, Format};
use crate::link::MediaLink;

/// Every progress line we asked for starts with this, so it never collides with yt-dlp's own text.
pub const PROGRESS_PREFIX: &str = "[orb] ";

/// `download:` selects the download stage. yt-dlp renders `%(...)j` as JSON; `progress` is the
/// progress-hook dict without `info_dict`, and `info.title` comes from the video's info.
pub const PROGRESS_TEMPLATE: &str = "download:[orb] {\"title\":%(info.title)j,\"progress\":%(progress)j}";

/// `.150B` cuts the title at 150 bytes so the path stays under Windows' 260 character limit.
pub const OUTPUT_TEMPLATE: &str = "%(title).150B [%(id)s].%(ext)s";

/// One download: which link, in which format, into which folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub link: MediaLink,
    pub format: Format,
    pub folder: PathBuf,
}

pub fn build_args(req: &Request, ffmpeg: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    let mut push = |s: &str| a.push(s.into());
    match req.format {
        Format::Video { max_height } => {
            let selector = match max_height {
                // `<=` is what makes a missing height fall back to the next lower one.
                Some(h) => format!("bv*[height<={h}][ext=mp4]+ba[ext=m4a]/bv*[height<={h}]+ba/b[height<={h}]/b"),
                None => "bv*[ext=mp4]+ba[ext=m4a]/bv*+ba/b".to_string(),
            };
            push("-f");
            push(&selector);
            push("--merge-output-format");
            push("mp4");
            push("--remux-video");
            push("mp4");
        }
        Format::Audio { codec } => {
            let selector = match codec {
                AudioCodec::M4a => "ba[ext=m4a]/ba/b",
                AudioCodec::Mp3 => "ba/b",
                AudioCodec::Opus => "ba[acodec^=opus]/ba/b",
            };
            push("-f");
            push(selector);
            push("-x");
            push("--audio-format");
            push(codec.ext());
            push("--audio-quality");
            push("0");
        }
    }
    for s in ["--newline", "--no-playlist", "--no-colors", "--progress-template", PROGRESS_TEMPLATE] {
        push(s);
    }
    a.push("--ffmpeg-location".into());
    a.push(ffmpeg.into());
    a.push("-P".into());
    a.push(req.folder.clone().into());
    a.push("-o".into());
    a.push(OUTPUT_TEMPLATE.into());
    // `--` ends the options, so no URL can ever be read as a flag.
    a.push("--".into());
    a.push(req.link.url().into());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(url: &str, format: Format) -> Vec<String> {
        let req = Request { link: MediaLink::parse(url).unwrap(), format, folder: PathBuf::from(r"C:\Users\me\Downloads\App") };
        build_args(&req, Path::new(r"C:\sidecar\ffmpeg.exe")).into_iter().map(|s| s.into_string().unwrap()).collect()
    }

    fn after<'a>(a: &'a [String], flag: &str) -> &'a str {
        let i = a.iter().position(|s| s == flag).unwrap_or_else(|| panic!("missing {flag}"));
        &a[i + 1]
    }

    #[test]
    fn video_1080_selects_mp4_at_most_1080_and_merges_to_mp4() {
        let a = args("https://youtu.be/dQw4w9WgXcQ", Format::default());
        assert_eq!(after(&a, "-f"), "bv*[height<=1080][ext=mp4]+ba[ext=m4a]/bv*[height<=1080]+ba/b[height<=1080]/b");
        assert_eq!(after(&a, "--merge-output-format"), "mp4");
        assert!(!a.contains(&"-x".to_string()));
    }

    #[test]
    fn audio_extracts_with_the_chosen_codec() {
        let a = args("https://soundcloud.com/a/b", Format::Audio { codec: AudioCodec::Mp3 });
        assert_eq!(after(&a, "-f"), "ba/b");
        assert!(a.contains(&"-x".to_string()));
        assert_eq!(after(&a, "--audio-format"), "mp3");
        let a = args("https://soundcloud.com/a/b", Format::Audio { codec: AudioCodec::M4a });
        assert_eq!(after(&a, "-f"), "ba[ext=m4a]/ba/b");
        assert_eq!(after(&a, "--audio-format"), "m4a");
    }

    #[test]
    fn common_flags_folder_ffmpeg_and_url_last() {
        let a = args("https://www.youtube.com/shorts/aBcDeFgHiJk", Format::Video { max_height: None });
        for flag in ["--newline", "--no-playlist"] {
            assert!(a.contains(&flag.to_string()), "{flag}");
        }
        assert_eq!(after(&a, "--progress-template"), PROGRESS_TEMPLATE);
        assert_eq!(after(&a, "--ffmpeg-location"), r"C:\sidecar\ffmpeg.exe");
        assert_eq!(after(&a, "-P"), r"C:\Users\me\Downloads\App");
        assert_eq!(after(&a, "-o"), OUTPUT_TEMPLATE);
        assert_eq!(&a[a.len() - 2..], ["--", "https://www.youtube.com/watch?v=aBcDeFgHiJk"]);
    }
}
