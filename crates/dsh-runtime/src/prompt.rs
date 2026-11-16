//! What a prompt is made of, and what this client will let through.
//!
//! The kernel's `session/prompt` takes `contentBlocks` — text is only one kind. Images are the
//! other one that matters, and the work of admitting them (copying bytes into the attachment
//! store, rewriting the block to a reference) happens inside the kernel. Our job is to hand it a
//! shape it accepts, and to refuse the shapes it would reject *before* a process spends a turn
//! discovering it.
//!
//! Validation lives here, in the runtime crate, rather than in the command layer: it is a
//! property of the protocol, and commands are supposed to stay thin.

use serde::{Deserialize, Serialize};

/// Bytes a single image may occupy, base64-decoded.
///
/// The kernel stores each image in its attachment directory and sends it to a model that has its
/// own per-image limits. 8 MB is generous for a screenshot and small enough that a mistake (a raw
/// 4K PNG, a video frame) is caught here instead of halfway through an upload.
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// Bytes all images in one prompt may occupy together.
pub const MAX_TOTAL_IMAGE_BYTES: usize = 24 * 1024 * 1024;

/// The image types the kernel's attachment store accepts.
///
/// Anything else is refused here with a clear message rather than failing inside the kernel with
/// a message about a media type nobody set out to use.
pub const ACCEPTED_IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

/// One block of a prompt.
///
/// Serialised straight onto the wire — the variant names are the protocol's `type` values, so a
/// change here is a protocol change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentBlock {
    /// Words from the user.
    Text { text: String },
    /// An image, base64-encoded. `name` is only a label; the kernel does not read it.
    Image {
        #[serde(rename = "mimeType")]
        mime_type: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

/// Why a prompt was refused before it was sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PromptError {
    #[error("a prompt needs at least one block")]
    Empty,
    #[error("a text block cannot be empty")]
    EmptyText,
    #[error("unsupported image type {found:?}; accepted: png, jpeg, webp, gif")]
    ImageType { found: String },
    #[error("image is empty")]
    EmptyImage,
    #[error("image is {0} bytes, over the {1} byte limit")]
    ImageTooLarge(usize, usize),
    #[error("images total {0} bytes, over the {1} byte budget")]
    ImagesTooLarge(usize, usize),
    #[error("image data is not valid base64")]
    NotBase64,
}

/// Check a batch of blocks the way the kernel would, but earlier and in plainer words.
pub fn validate(blocks: &[ContentBlock]) -> Result<(), PromptError> {
    if blocks.is_empty() {
        return Err(PromptError::Empty);
    }
    let mut total = 0usize;
    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                if text.trim().is_empty() {
                    return Err(PromptError::EmptyText);
                }
            }
            ContentBlock::Image {
                mime_type, data, ..
            } => {
                let normalized = mime_type.trim().to_ascii_lowercase();
                if !ACCEPTED_IMAGE_TYPES.contains(&normalized.as_str()) {
                    return Err(PromptError::ImageType {
                        found: mime_type.clone(),
                    });
                }
                if data.is_empty() {
                    return Err(PromptError::EmptyImage);
                }
                let bytes = decoded_len(data).ok_or(PromptError::NotBase64)?;
                if bytes > MAX_IMAGE_BYTES {
                    return Err(PromptError::ImageTooLarge(bytes, MAX_IMAGE_BYTES));
                }
                total += bytes;
                if total > MAX_TOTAL_IMAGE_BYTES {
                    return Err(PromptError::ImagesTooLarge(total, MAX_TOTAL_IMAGE_BYTES));
                }
            }
        }
    }
    Ok(())
}

/// The decoded size of a base64 string, or `None` if it is not base64.
///
/// Deliberately does not decode: the point is to reject a bad payload without allocating it.
fn decoded_len(data: &str) -> Option<usize> {
    let trimmed = data.trim_end_matches('=');
    if trimmed.is_empty() {
        return Some(0);
    }
    let padding = data.len() - trimmed.len();
    if padding > 2 {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
    {
        return None;
    }
    // Every four base64 characters carry three bytes.
    Some(trimmed.len() / 4 * 3 + (trimmed.len() % 4) * 3 / 4)
}

/// A prompt that is just words — the common case, kept short so callers do not build a vector
/// by hand every time.
pub fn text_block(text: impl Into<String>) -> ContentBlock {
    ContentBlock::Text { text: text.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(mime: &str, data: &str) -> ContentBlock {
        ContentBlock::Image {
            mime_type: mime.to_owned(),
            data: data.to_owned(),
            name: None,
        }
    }

    #[test]
    fn a_text_prompt_serialises_the_way_the_kernel_expects() {
        let json = serde_json::to_value(text_block("你好")).expect("serialise");
        assert_eq!(json, serde_json::json!({"type": "text", "text": "你好"}));
    }

    #[test]
    fn an_image_block_carries_mime_type_and_data() {
        let json = serde_json::to_value(image("image/png", "iVBORw0KGgo=")).expect("serialise");
        assert_eq!(json["type"], "image");
        assert_eq!(json["mimeType"], "image/png");
        assert_eq!(json["data"], "iVBORw0KGgo=");
        assert!(json.get("name").is_none(), "optional keys are omitted");
    }

    #[test]
    fn an_empty_prompt_is_refused() {
        assert_eq!(validate(&[]), Err(PromptError::Empty));
        assert_eq!(validate(&[text_block("   ")]), Err(PromptError::EmptyText));
    }

    #[test]
    fn an_unsupported_image_type_names_what_is_accepted() {
        let error = validate(&[image("image/tiff", "AAAA")]).expect_err("refused");
        let message = error.to_string();
        assert!(message.contains("image/tiff"), "{message}");
        assert!(
            message.contains("png"),
            "the caller needs to know the options: {message}"
        );
    }

    #[test]
    fn a_payload_that_is_not_base64_is_refused_without_decoding_it() {
        assert_eq!(
            validate(&[image("image/png", "not base64!")]),
            Err(PromptError::NotBase64)
        );
    }

    #[test]
    fn the_size_limit_counts_decoded_bytes_not_characters() {
        // 4 base64 characters are 3 bytes: just under the limit passes, just over fails.
        let at_limit = "A".repeat(MAX_IMAGE_BYTES / 3 * 4);
        assert!(validate(&[image("image/png", &at_limit)]).is_ok());
        let over = "A".repeat(MAX_IMAGE_BYTES / 3 * 4 + 4);
        assert!(matches!(
            validate(&[image("image/png", &over)]),
            Err(PromptError::ImageTooLarge(..))
        ));
    }

    #[test]
    fn text_and_an_image_can_be_mixed() {
        assert!(validate(&[text_block("看看这个"), image("image/jpeg", "AAAA")]).is_ok());
    }
}
