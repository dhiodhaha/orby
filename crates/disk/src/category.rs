//! File type categories for the Types tab, decided by extension alone.

use std::ffi::OsStr;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Video,
    AppsAndGames,
    Archives,
    Images,
    Documents,
    Other,
}

/// One slice of the stacked type bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeShare {
    pub category: Category,
    pub bytes: u64,
    pub files: u64,
}

// No "ts" or "mts": on a PC those are TypeScript far more often than MPEG streams.
const VIDEO: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "3gp", "m2ts", "vob"];
const APPS: &[&str] = &[
    "exe",
    "msi",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "dll",
    "sys",
    "apk",
    "xapk",
    "pak",
    "vpk",
    "bsa",
    "ba2",
    "ucas",
    "utoc",
    "upk",
    "forge",
];
const ARCHIVES: &[&str] =
    &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "lz", "lzma", "cab", "iso", "img", "vhd", "vhdx", "wim", "dmg"];
const IMAGES: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "tif", "tiff", "webp", "heic", "heif", "avif", "svg", "ico", "psd", "raw", "dng", "cr2", "cr3",
    "nef", "arw",
];
const DOCUMENTS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "md", "csv", "epub", "one", "pages", "numbers",
    "key",
];

impl Category {
    pub const ALL: [Category; 6] = [Self::Video, Self::AppsAndGames, Self::Archives, Self::Images, Self::Documents, Self::Other];

    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::AppsAndGames => "Apps and games",
            Self::Archives => "Archives",
            Self::Images => "Images",
            Self::Documents => "Documents",
            Self::Other => "Other",
        }
    }

    /// Category of a file name, case-insensitive on the extension.
    pub fn of(name: &OsStr) -> Category {
        let Some(ext) = Path::new(name).extension().and_then(OsStr::to_str) else { return Self::Other };
        let ext = ext.to_ascii_lowercase();
        let ext = ext.as_str();
        [(VIDEO, Self::Video), (APPS, Self::AppsAndGames), (ARCHIVES, Self::Archives), (IMAGES, Self::Images), (DOCUMENTS, Self::Documents)]
            .into_iter()
            .find(|(list, _)| list.contains(&ext))
            .map_or(Self::Other, |(_, c)| c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(n: &str) -> Category {
        Category::of(OsStr::new(n))
    }

    #[test]
    fn spec_examples_land_in_their_category() {
        assert_eq!(of("Win11_24H2_x64.iso"), Category::Archives);
        assert_eq!(of("holiday.MP4"), Category::Video);
        assert_eq!(of("setup.exe"), Category::AppsAndGames);
        assert_eq!(of("IMG_0001.HEIC"), Category::Images);
        assert_eq!(of("thesis.docx"), Category::Documents);
        assert_eq!(of("song.flac"), Category::Other);
        assert_eq!(of("README"), Category::Other);
        assert_eq!(of(".gitignore"), Category::Other);
    }

    #[test]
    fn typescript_sources_are_not_video() {
        assert_eq!(of("main.ts"), Category::Other);
        assert_eq!(of("index.d.mts"), Category::Other);
        assert_eq!(of("clip.m2ts"), Category::Video);
    }

    #[test]
    fn all_is_in_discriminant_order() {
        for (i, c) in Category::ALL.iter().enumerate() {
            assert_eq!(*c as usize, i);
        }
    }
}
