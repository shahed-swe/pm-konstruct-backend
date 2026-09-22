//! Media types, limits and content validation — domain-rules R10.
//!
//! The declared MIME type is attacker-controlled, so it is never trusted on its
//! own: every upload is checked against the file's actual magic bytes. A `.jpg`
//! that is really an executable must be rejected.
//!
//! Ported from `middleware/jobPhotoUpload.ts`. The signature checks are
//! byte-for-byte the legacy ones; the limits and the MIME-to-extension map are
//! unchanged.

use crate::{DomainError, DomainResult};

/// 20 MB. Photos only.
pub const MAX_PHOTO_BYTES: u64 = 20 * 1024 * 1024;
/// 500 MB. Videos only.
pub const MAX_VIDEO_BYTES: u64 = 500 * 1024 * 1024;
/// Files accepted in a single upload.
pub const MAX_FILES_PER_UPLOAD: usize = 10;
/// Photos attachable to one form email.
pub const MAX_FORM_EMAIL_PHOTOS: usize = 50;

/// How a row in `diary_media` / `job_media` is classified.
///
/// Constrained by `diary_media_file_type_check` and `job_media_file_type_check`
/// (migration 0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    Photo,
    Video,
    Document,
}

impl FileType {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "photo" => Some(Self::Photo),
            "video" => Some(Self::Video),
            "document" => Some(Self::Document),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Photo => "photo",
            Self::Video => "video",
            Self::Document => "document",
        }
    }
}

/// An accepted upload type: its MIME string, stored extension and class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaKind {
    pub mime: &'static str,
    pub extension: &'static str,
    pub file_type: FileType,
}

impl MediaKind {
    /// The per-file size ceiling. The legacy code chose by
    /// `mimetype.startsWith("video/")`, which this reproduces.
    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        match self.file_type {
            FileType::Video => MAX_VIDEO_BYTES,
            FileType::Photo | FileType::Document => MAX_PHOTO_BYTES,
        }
    }
}

/// Every MIME type the legacy uploader accepted, in its original order.
///
/// Anything absent is rejected outright: the map doubles as the allowlist, so
/// adding a type is a deliberate edit here rather than an accident elsewhere.
pub const ACCEPTED: &[MediaKind] = &[
    MediaKind {
        mime: "image/jpeg",
        extension: "jpg",
        file_type: FileType::Photo,
    },
    MediaKind {
        mime: "image/jpg",
        extension: "jpg",
        file_type: FileType::Photo,
    },
    MediaKind {
        mime: "image/png",
        extension: "png",
        file_type: FileType::Photo,
    },
    MediaKind {
        mime: "image/gif",
        extension: "gif",
        file_type: FileType::Photo,
    },
    MediaKind {
        mime: "image/webp",
        extension: "webp",
        file_type: FileType::Photo,
    },
    MediaKind {
        mime: "image/heic",
        extension: "heic",
        file_type: FileType::Photo,
    },
    MediaKind {
        mime: "image/heif",
        extension: "heif",
        file_type: FileType::Photo,
    },
    MediaKind {
        mime: "video/mp4",
        extension: "mp4",
        file_type: FileType::Video,
    },
    MediaKind {
        mime: "video/quicktime",
        extension: "mov",
        file_type: FileType::Video,
    },
    MediaKind {
        mime: "video/webm",
        extension: "webm",
        file_type: FileType::Video,
    },
    MediaKind {
        mime: "video/x-msvideo",
        extension: "avi",
        file_type: FileType::Video,
    },
    MediaKind {
        mime: "video/avi",
        extension: "avi",
        file_type: FileType::Video,
    },
];

#[must_use]
pub fn kind_for(mime: &str) -> Option<MediaKind> {
    ACCEPTED.iter().copied().find(|k| k.mime == mime)
}

/// Whether `head` (the first bytes of the file) matches what `mime` claims.
///
/// Needs at least 12 bytes to decide on the container formats; a shorter file
/// is rejected rather than given the benefit of the doubt.
#[must_use]
pub fn signature_matches(mime: &str, head: &[u8]) -> bool {
    /// `b` reads a byte, returning a sentinel that cannot match when the file
    /// is too short.
    fn b(head: &[u8], i: usize) -> u16 {
        head.get(i).map_or(0x100, |v| u16::from(*v))
    }

    match mime {
        // FF D8 FF
        "image/jpeg" | "image/jpg" => {
            b(head, 0) == 0xff && b(head, 1) == 0xd8 && b(head, 2) == 0xff
        }
        // 89 50 4E 47 0D 0A 1A 0A
        "image/png" => {
            b(head, 0) == 0x89
                && b(head, 1) == 0x50
                && b(head, 2) == 0x4e
                && b(head, 3) == 0x47
                && b(head, 4) == 0x0d
                && b(head, 5) == 0x0a
                && b(head, 6) == 0x1a
                && b(head, 7) == 0x0a
        }
        // "GIF8" -- covers both 87a and 89a
        "image/gif" => {
            b(head, 0) == 0x47 && b(head, 1) == 0x49 && b(head, 2) == 0x46 && b(head, 3) == 0x38
        }
        // RIFF....WEBP
        "image/webp" => {
            b(head, 0) == 0x52
                && b(head, 1) == 0x49
                && b(head, 2) == 0x46
                && b(head, 3) == 0x46
                && b(head, 8) == 0x57
                && b(head, 9) == 0x45
                && b(head, 10) == 0x42
                && b(head, 11) == 0x50
        }
        // ISO base media: "ftyp" at offset 4, then a HEIF-family brand.
        "image/heic" | "image/heif" => {
            is_iso_base_media(head)
                && matches!(iso_brand(head), Some(brand) if is_heif_brand(brand))
        }
        // Same "ftyp" box. The legacy code did not check the brand for video,
        // and neither does this -- narrowing it would reject files that upload
        // successfully today.
        "video/mp4" | "video/quicktime" => is_iso_base_media(head),
        // EBML header
        "video/webm" => {
            b(head, 0) == 0x1a && b(head, 1) == 0x45 && b(head, 2) == 0xdf && b(head, 3) == 0xa3
        }
        // RIFF....AVI<space>
        "video/x-msvideo" | "video/avi" => {
            b(head, 0) == 0x52
                && b(head, 1) == 0x49
                && b(head, 2) == 0x46
                && b(head, 3) == 0x46
                && b(head, 8) == 0x41
                && b(head, 9) == 0x56
                && b(head, 10) == 0x49
                && b(head, 11) == 0x20
        }
        _ => false,
    }
}

/// "ftyp" at offset 4.
fn is_iso_base_media(head: &[u8]) -> bool {
    head.len() >= 8 && &head[4..8] == b"ftyp"
}

/// The four-character brand following "ftyp".
fn iso_brand(head: &[u8]) -> Option<&[u8]> {
    head.get(8..12)
}

fn is_heif_brand(brand: &[u8]) -> bool {
    // heic/heix/hevc/hevx are HEVC-coded; mif1/msf1 are the generic HEIF
    // brands iPhones also emit.
    matches!(
        brand,
        b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1"
    )
}

/// How many bytes `signature_matches` needs.
pub const SIGNATURE_PROBE_BYTES: usize = 16;

/// A validated upload request, before the bytes are stored.
#[derive(Debug, Clone)]
pub struct UploadRequest {
    pub original_name: String,
    pub mime: String,
    pub size_bytes: u64,
}

/// The outcome of validating one upload.
#[derive(Debug, Clone)]
pub struct ValidatedUpload {
    pub original_name: String,
    pub kind: MediaKind,
    pub size_bytes: u64,
    /// `<uuid>.<ext>` -- the name the object is stored under. Never the
    /// user's filename, which is attacker-controlled and may contain path
    /// separators or be absurdly long.
    pub stored_name: String,
}

/// Validates the declared type and size. Content is checked separately by
/// [`signature_matches`], once bytes are available.
pub fn validate_upload(req: &UploadRequest) -> DomainResult<ValidatedUpload> {
    let kind = kind_for(&req.mime).ok_or_else(|| {
        DomainError::invalid(
            "mimeType",
            format!("{} is not an accepted photo or video type", req.mime),
        )
    })?;

    if req.size_bytes == 0 {
        return Err(DomainError::invalid("size", "the file is empty"));
    }
    if req.size_bytes > kind.max_bytes() {
        return Err(DomainError::invalid(
            "size",
            "Each photo must be 20 MB or smaller, or each video must be 500 MB or smaller.",
        ));
    }
    if req.original_name.trim().is_empty() {
        return Err(DomainError::invalid("fileName", "is required"));
    }

    Ok(ValidatedUpload {
        original_name: sanitise_original_name(&req.original_name),
        kind,
        size_bytes: req.size_bytes,
        stored_name: format!("{}.{}", uuid_v4(), kind.extension),
    })
}

/// Checks a batch against the per-request file count.
pub fn validate_batch_size(count: usize, limit: usize) -> DomainResult<()> {
    if count == 0 {
        return Err(DomainError::invalid(
            "files",
            "at least one file is required",
        ));
    }
    if count > limit {
        return Err(DomainError::invalid(
            "files",
            format!("Choose up to {limit} photos or videos at a time."),
        ));
    }
    Ok(())
}

/// Keeps the display name usable without letting it reach the filesystem.
///
/// `original_name` is `varchar(255)` and is rendered in the UI and in emails,
/// so path separators, control characters and over-long names are stripped.
/// The stored object name is a UUID regardless, so this is about display, not
/// about path traversal -- but a name containing `../` shown to a user is
/// still worth removing.
fn sanitise_original_name(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return "upload".to_string();
    }
    // varchar(255) in both media tables.
    cleaned.chars().take(255).collect()
}

/// Random v4 UUID for the stored object name.
///
/// Uses the `uuid` crate rather than anything hand-rolled: object keys are the
/// only thing between a leaked URL and someone else's photos, so the randomness
/// needs to come from a CSPRNG that is someone else's job to get right.
fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Builds the storage key for a stored file.
///
/// Tenant-prefixed so a bucket policy can enforce isolation as a third layer,
/// after `TenantScope` and RLS. The stored name is already a UUID, so the
/// prefix is for operability -- listing one company's objects, or deleting
/// them on offboarding -- rather than for secrecy.
///
/// It lives in the domain because both the storage adapter and the use cases
/// need it, and two copies of a key-naming rule would drift apart without
/// anything failing until objects went missing.
#[must_use]
pub fn object_key(company_id: i32, entity: &str, entity_id: i32, stored_name: &str) -> String {
    format!("{company_id}/{entity}/{entity_id}/{stored_name}")
}

/// May this user delete media somebody else uploaded?
///
/// Office staff may remove only their own uploads. Managers and supervisors
/// run the site and may remove anyone's -- a photo of defective work is not
/// the photographer's property to keep.
///
/// The legacy code applied this by passing the uploader id only when the role
/// was OFFICE; expressing it as a rule keeps the condition in one place and
/// out of three separate call sites.
#[must_use]
pub fn may_delete_media(
    role: crate::access::Role,
    viewer: crate::ids::UserId,
    uploaded_by: Option<crate::ids::UserId>,
) -> bool {
    if role != crate::access::Role::Office {
        return true;
    }
    // An upload with no recorded uploader predates the column. Office staff
    // cannot claim it, because there is nothing to match them against.
    uploaded_by == Some(viewer)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::access::Role;
    use crate::ids::UserId;

    #[test]
    fn office_staff_may_only_delete_their_own_uploads() {
        let me = UserId(4);
        assert!(may_delete_media(Role::Office, me, Some(me)));
        assert!(!may_delete_media(Role::Office, me, Some(UserId(2))));
    }

    #[test]
    fn office_staff_cannot_claim_an_upload_with_no_uploader() {
        // A row predating the column has nobody to match against, so it stays
        // out of reach rather than becoming deletable by everyone.
        assert!(!may_delete_media(Role::Office, UserId(4), None));
    }

    #[test]
    fn managers_and_supervisors_may_delete_anyones() {
        for role in [Role::Manager, Role::Supervisor] {
            assert!(may_delete_media(role, UserId(1), Some(UserId(99))));
            assert!(may_delete_media(role, UserId(1), None));
        }
    }

    fn req(mime: &str, size: u64) -> UploadRequest {
        UploadRequest {
            original_name: "site photo.jpg".into(),
            mime: mime.into(),
            size_bytes: size,
        }
    }

    // -- the allowlist -------------------------------------------------------
    #[test]
    fn every_legacy_mime_type_is_still_accepted() {
        for mime in [
            "image/jpeg",
            "image/jpg",
            "image/png",
            "image/gif",
            "image/webp",
            "image/heic",
            "image/heif",
            "video/mp4",
            "video/quicktime",
            "video/webm",
            "video/x-msvideo",
            "video/avi",
        ] {
            assert!(kind_for(mime).is_some(), "{mime} should be accepted");
        }
    }

    #[test]
    fn dangerous_types_are_rejected() {
        for mime in [
            "application/x-msdownload",
            "text/html",
            "image/svg+xml", // scriptable
            "application/pdf",
            "",
        ] {
            assert!(kind_for(mime).is_none(), "{mime} must not be accepted");
            assert!(validate_upload(&req(mime, 1024)).is_err());
        }
    }

    // -- size limits ---------------------------------------------------------
    #[test]
    fn photo_and_video_have_different_ceilings() {
        assert_eq!(kind_for("image/jpeg").unwrap().max_bytes(), MAX_PHOTO_BYTES);
        assert_eq!(kind_for("video/mp4").unwrap().max_bytes(), MAX_VIDEO_BYTES);
    }

    #[test]
    fn a_photo_over_twenty_megabytes_is_rejected() {
        assert!(validate_upload(&req("image/jpeg", MAX_PHOTO_BYTES)).is_ok());
        assert!(validate_upload(&req("image/jpeg", MAX_PHOTO_BYTES + 1)).is_err());
    }

    #[test]
    fn a_video_may_be_much_larger_than_a_photo() {
        // 100 MB is fine as a video and impossible as a photo.
        let big = 100 * 1024 * 1024;
        assert!(validate_upload(&req("video/mp4", big)).is_ok());
        assert!(validate_upload(&req("image/jpeg", big)).is_err());
        assert!(validate_upload(&req("video/mp4", MAX_VIDEO_BYTES + 1)).is_err());
    }

    #[test]
    fn an_empty_file_is_rejected() {
        assert!(validate_upload(&req("image/jpeg", 0)).is_err());
    }

    #[test]
    fn batch_size_matches_the_legacy_limits() {
        assert!(validate_batch_size(10, MAX_FILES_PER_UPLOAD).is_ok());
        assert!(validate_batch_size(11, MAX_FILES_PER_UPLOAD).is_err());
        assert!(validate_batch_size(0, MAX_FILES_PER_UPLOAD).is_err());
        assert!(validate_batch_size(50, MAX_FORM_EMAIL_PHOTOS).is_ok());
        assert!(validate_batch_size(51, MAX_FORM_EMAIL_PHOTOS).is_err());
    }

    // -- stored names --------------------------------------------------------
    #[test]
    fn the_stored_name_is_a_uuid_not_the_users_filename() {
        let v = validate_upload(&req("image/png", 100)).unwrap();
        assert!(v.stored_name.ends_with(".png"));
        assert!(!v.stored_name.contains("site photo"));
        assert_eq!(v.stored_name.len(), 36 + 4, "uuid + .png");
    }

    #[test]
    fn stored_names_do_not_repeat() {
        let a = validate_upload(&req("image/png", 100)).unwrap().stored_name;
        let b = validate_upload(&req("image/png", 100)).unwrap().stored_name;
        assert_ne!(a, b, "object keys must be unguessable and unique");
    }

    #[test]
    fn path_separators_are_stripped_from_the_display_name() {
        let mut r = req("image/jpeg", 100);
        r.original_name = "../../etc/passwd".into();
        let v = validate_upload(&r).unwrap();
        assert!(!v.original_name.contains('/'));
        assert!(!v.original_name.contains('\\'));
    }

    #[test]
    fn control_characters_are_stripped() {
        let mut r = req("image/jpeg", 100);
        r.original_name = "photo\r\nX-Injected: 1.jpg".into();
        let v = validate_upload(&r).unwrap();
        assert!(!v.original_name.contains('\n'));
        assert!(!v.original_name.contains('\r'));
    }

    #[test]
    fn over_long_names_are_truncated_to_the_column_width() {
        let mut r = req("image/jpeg", 100);
        r.original_name = "a".repeat(500);
        let v = validate_upload(&r).unwrap();
        assert_eq!(v.original_name.chars().count(), 255);
    }

    #[test]
    fn a_name_that_sanitises_to_nothing_gets_a_placeholder() {
        let mut r = req("image/jpeg", 100);
        r.original_name = "///".into();
        assert_eq!(validate_upload(&r).unwrap().original_name, "upload");
    }

    // -- magic bytes ---------------------------------------------------------
    fn jpeg() -> Vec<u8> {
        let mut v = vec![0xff, 0xd8, 0xff, 0xe0];
        v.resize(16, 0);
        v
    }
    fn png() -> Vec<u8> {
        let mut v = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        v.resize(16, 0);
        v
    }
    fn riff(tag: &[u8]) -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0".to_vec();
        v.extend_from_slice(tag);
        v.resize(16, 0);
        v
    }
    fn ftyp(brand: &[u8]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 0x20];
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(brand);
        v.resize(16, 0);
        v
    }

    #[test]
    fn real_signatures_are_accepted() {
        assert!(signature_matches("image/jpeg", &jpeg()));
        assert!(signature_matches("image/jpg", &jpeg()));
        assert!(signature_matches("image/png", &png()));
        assert!(signature_matches(
            "image/gif",
            b"GIF89a\0\0\0\0\0\0\0\0\0\0"
        ));
        assert!(signature_matches(
            "image/gif",
            b"GIF87a\0\0\0\0\0\0\0\0\0\0"
        ));
        assert!(signature_matches("image/webp", &riff(b"WEBP")));
        assert!(signature_matches("video/x-msvideo", &riff(b"AVI ")));
        assert!(signature_matches("video/avi", &riff(b"AVI ")));
        assert!(signature_matches("video/mp4", &ftyp(b"isom")));
        assert!(signature_matches("video/quicktime", &ftyp(b"qt  ")));
        assert!(signature_matches(
            "video/webm",
            b"\x1a\x45\xdf\xa3\0\0\0\0\0\0\0\0\0\0\0\0"
        ));
        for brand in [b"heic", b"heix", b"mif1", b"msf1"] {
            assert!(signature_matches("image/heic", &ftyp(brand)), "{brand:?}");
        }
    }

    #[test]
    fn an_executable_renamed_to_jpg_is_rejected() {
        // Mach-O and PE headers -- the whole point of checking content.
        assert!(!signature_matches(
            "image/jpeg",
            b"\xcf\xfa\xed\xfe\0\0\0\0\0\0\0\0\0\0\0\0"
        ));
        assert!(!signature_matches(
            "image/jpeg",
            b"MZ\x90\x00\0\0\0\0\0\0\0\0\0\0\0\0"
        ));
        assert!(!signature_matches("image/png", b"#!/bin/sh\n\0\0\0\0\0\0"));
    }

    #[test]
    fn a_png_declared_as_jpeg_is_rejected() {
        assert!(!signature_matches("image/jpeg", &png()));
        assert!(!signature_matches("image/png", &jpeg()));
    }

    #[test]
    fn a_riff_container_with_the_wrong_tag_is_rejected() {
        // RIFF alone is not enough: WAVE is a valid RIFF file and not an image.
        assert!(!signature_matches("image/webp", &riff(b"WAVE")));
        assert!(!signature_matches("video/x-msvideo", &riff(b"WEBP")));
    }

    #[test]
    fn heif_requires_a_heif_brand_not_just_ftyp() {
        // An mp4 is also an ftyp box; declaring it image/heic must fail.
        assert!(!signature_matches("image/heic", &ftyp(b"isom")));
        assert!(!signature_matches("image/heif", &ftyp(b"mp42")));
    }

    #[test]
    fn a_truncated_file_never_passes() {
        for mime in [
            "image/jpeg",
            "image/png",
            "image/webp",
            "video/mp4",
            "image/heic",
        ] {
            assert!(!signature_matches(mime, b""), "{mime} on empty input");
            assert!(!signature_matches(mime, b"\xff"), "{mime} on 1 byte");
            assert!(!signature_matches(mime, b"\xff\xd8"), "{mime} on 2 bytes");
        }
    }

    #[test]
    fn an_unknown_mime_never_matches_any_content() {
        assert!(!signature_matches("application/x-msdownload", &jpeg()));
        assert!(!signature_matches("", &jpeg()));
    }
}
