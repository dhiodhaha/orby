//! What to save: video up to a height, or audio in one codec.

use crate::link::MediaLink;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Video,
    Audio,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    /// Keeps the source quality (YouTube serves AAC in M4A, so no re-encode).
    #[default]
    M4a,
    Mp3,
    Opus,
}

impl AudioCodec {
    /// The value yt-dlp takes for `--audio-format`, also the file extension.
    pub fn ext(self) -> &'static str {
        match self {
            AudioCodec::M4a => "m4a",
            AudioCodec::Mp3 => "mp3",
            AudioCodec::Opus => "opus",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioCodec::M4a => "M4A",
            AudioCodec::Mp3 => "MP3",
            AudioCodec::Opus => "Opus",
        }
    }
}

pub const DEFAULT_MAX_HEIGHT: u32 = 1080;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// MP4 at most this tall, taking the next lower height when it is missing. `None` is "Best".
    Video { max_height: Option<u32> },
    Audio { codec: AudioCodec },
}

impl Default for Format {
    fn default() -> Self {
        Format::Video { max_height: Some(DEFAULT_MAX_HEIGHT) }
    }
}

impl Format {
    pub fn kind(self) -> MediaKind {
        match self {
            Format::Video { .. } => MediaKind::Video,
            Format::Audio { .. } => MediaKind::Audio,
        }
    }

    /// The muted text in the offer pill and the queue row: "1080p MP4", "Best MP4", "M4A".
    pub fn label(self) -> String {
        match self {
            Format::Video { max_height: Some(h) } => format!("{h}p MP4"),
            Format::Video { max_height: None } => "Best MP4".to_string(),
            Format::Audio { codec } => codec.label().to_string(),
        }
    }
}

/// The Settings panel's choices ("Save copied links as", video quality, audio format).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatPrefs {
    pub save_as: MediaKind,
    pub max_height: Option<u32>,
    pub audio: AudioCodec,
}

impl Default for FormatPrefs {
    fn default() -> Self {
        FormatPrefs { save_as: MediaKind::Video, max_height: Some(DEFAULT_MAX_HEIGHT), audio: AudioCodec::M4a }
    }
}

impl FormatPrefs {
    pub fn video(self) -> Format {
        Format::Video { max_height: self.max_height }
    }

    /// What the `A` key on the pill saves.
    pub fn audio(self) -> Format {
        Format::Audio { codec: self.audio }
    }

    /// What Save does for this link. Music sites always start as audio.
    pub fn for_link(self, link: &MediaLink) -> Format {
        if self.save_as == MediaKind::Audio || link.site().default_kind() == MediaKind::Audio {
            self.audio()
        } else {
            self.video()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_settings_bold_chips() {
        assert_eq!(Format::default(), Format::Video { max_height: Some(1080) });
        assert_eq!(Format::default().label(), "1080p MP4");
        assert_eq!(FormatPrefs::default().audio(), Format::Audio { codec: AudioCodec::M4a });
    }

    #[test]
    fn labels_for_the_format_menu() {
        assert_eq!(Format::Video { max_height: Some(720) }.label(), "720p MP4");
        assert_eq!(Format::Video { max_height: None }.label(), "Best MP4");
        assert_eq!(Format::Audio { codec: AudioCodec::M4a }.label(), "M4A");
        assert_eq!(Format::Audio { codec: AudioCodec::Mp3 }.label(), "MP3");
        assert_eq!(Format::Audio { codec: AudioCodec::Opus }.label(), "Opus");
    }

    #[test]
    fn music_sites_start_as_audio_even_when_settings_say_video() {
        let prefs = FormatPrefs { audio: AudioCodec::Mp3, ..FormatPrefs::default() };
        let yt = MediaLink::parse("https://youtu.be/dQw4w9WgXcQ").unwrap();
        let music = MediaLink::parse("https://music.youtube.com/watch?v=dQw4w9WgXcQ").unwrap();
        let sc = MediaLink::parse("https://soundcloud.com/a/b").unwrap();
        assert_eq!(prefs.for_link(&yt), Format::Video { max_height: Some(1080) });
        assert_eq!(prefs.for_link(&music), Format::Audio { codec: AudioCodec::Mp3 });
        assert_eq!(prefs.for_link(&sc).kind(), MediaKind::Audio);
    }

    #[test]
    fn audio_only_setting_applies_to_video_sites() {
        let prefs = FormatPrefs { save_as: MediaKind::Audio, ..FormatPrefs::default() };
        let yt = MediaLink::parse("https://youtu.be/dQw4w9WgXcQ").unwrap();
        assert_eq!(prefs.for_link(&yt), Format::Audio { codec: AudioCodec::M4a });
    }
}
