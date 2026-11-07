//! Meeting join links: Google Meet, Zoom and Microsoft Teams.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkKind {
    Meet,
    Zoom,
    Teams,
}

impl LinkKind {
    /// The call type as the Today row shows it ("Work · Meet").
    pub fn label(self) -> &'static str {
        match self {
            LinkKind::Meet => "Meet",
            LinkKind::Zoom => "Zoom",
            LinkKind::Teams => "Teams",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeetingLink {
    pub kind: LinkKind,
    pub url: String,
}

/// The first join link in the texts, searched in the order given (URL, then LOCATION, then DESCRIPTION).
pub fn find_meeting_link<'a>(texts: impl IntoIterator<Item = &'a str>) -> Option<MeetingLink> {
    texts
        .into_iter()
        .flat_map(urls)
        .find_map(|url| classify(url).map(|kind| MeetingLink { kind, url: url.to_string() }))
}

/// Every http(s) URL in a text. A URL ends at whitespace, a quote or an angle bracket (Outlook
/// writes `text<https://...>`), and loses trailing sentence punctuation.
fn urls(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = [rest.find("https://"), rest.find("http://")].into_iter().flatten().min() {
        let tail = &rest[i..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'')).unwrap_or(tail.len());
        out.push(tail[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']']));
        rest = &tail[end..];
    }
    out
}

fn classify(url: &str) -> Option<LinkKind> {
    let lower = url.to_ascii_lowercase();
    let after = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let split = after.find(['/', '?', '#']).unwrap_or(after.len());
    let (host, path) = after.split_at(split);
    let host = host.split(':').next().unwrap_or(host);
    let zoom_host = host == "zoom.us" || host.ends_with(".zoom.us");
    let has = |prefixes: &[&str]| prefixes.iter().any(|p| path.starts_with(p) && path.len() > p.len());
    if host == "meet.google.com" && has(&["/"]) {
        Some(LinkKind::Meet)
    } else if zoom_host && has(&["/j/", "/s/", "/w/", "/my/"]) {
        Some(LinkKind::Zoom)
    } else if (host == "teams.microsoft.com" && has(&["/l/meetup-join/", "/meet/"]))
        || (host == "teams.live.com" && has(&["/meet/"]))
    {
        Some(LinkKind::Teams)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(text: &str) -> Option<LinkKind> {
        find_meeting_link([text]).map(|l| l.kind)
    }

    #[test]
    fn recognises_the_three_providers() {
        assert_eq!(kind("https://meet.google.com/abc-defg-hij"), Some(LinkKind::Meet));
        assert_eq!(kind("Zoom: https://us02web.zoom.us/j/81234567890?pwd=x"), Some(LinkKind::Zoom));
        assert_eq!(kind("https://zoom.us/my/dhio"), Some(LinkKind::Zoom));
        assert_eq!(
            kind("Join<https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=%7b%7d>"),
            Some(LinkKind::Teams)
        );
        assert_eq!(kind("https://teams.live.com/meet/9876"), Some(LinkKind::Teams));
    }

    #[test]
    fn ignores_lookalikes_and_bare_hosts() {
        assert_eq!(kind("https://meet.google.com/"), None);
        assert_eq!(kind("https://zoom.us/pricing"), None);
        assert_eq!(kind("https://notzoom.us/j/1"), None);
        assert_eq!(kind("https://teams.microsoft.com/downloads"), None);
        assert_eq!(kind("https://example.com/?u=https"), None);
        assert_eq!(kind("Ruang Rapat 3"), None);
    }

    #[test]
    fn keeps_the_link_exact_and_strips_trailing_punctuation() {
        let link = find_meeting_link(["Call at https://meet.google.com/abc-defg-hij."]).unwrap();
        assert_eq!(link.url, "https://meet.google.com/abc-defg-hij");
        let link = find_meeting_link(["(https://us02web.zoom.us/j/1?pwd=AbC)"]).unwrap();
        assert_eq!(link.url, "https://us02web.zoom.us/j/1?pwd=AbC");
    }

    #[test]
    fn earlier_sources_win() {
        let link = find_meeting_link(["https://zoom.us/j/1", "https://meet.google.com/abc-defg-hij"]).unwrap();
        assert_eq!(link.kind, LinkKind::Zoom);
        let link = find_meeting_link(["https://example.com", "https://meet.google.com/abc-defg-hij"]).unwrap();
        assert_eq!(link.kind, LinkKind::Meet);
    }
}
