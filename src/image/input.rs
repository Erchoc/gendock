//! Read, validate and snapshot user-supplied edit inputs without changing their files.
use crate::models::ModelError;
use anyhow::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub struct Input {
    pub source: PathBuf,
    pub bytes: Vec<u8>,
    pub extension: &'static str,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
}
impl Input {
    pub fn read(path: &Path) -> Result<Self> {
        let invalid = |code, message, next| ModelError {
            code,
            message,
            retryable: false,
            next_step: next,
            details: json!({"input":path}),
        };
        let source = path.canonicalize().map_err(|_| {
            invalid(
                "input_not_found",
                "The input image is unavailable.",
                "Check --input and reconnect any external disk before retrying.",
            )
        })?;
        let mut bytes = Vec::new();
        std::fs::File::open(&source)?
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(invalid(
                "invalid_input_image",
                "The input image exceeds 64 MiB.",
                "Use a smaller PNG, JPEG or WebP image.",
            )
            .into());
        }
        let mut reader = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|_| {
                invalid(
                    "invalid_input_image",
                    "The input is not a readable image.",
                    "Use a valid PNG, JPEG or WebP image.",
                )
            })?;
        let extension = match reader.format() {
            Some(image::ImageFormat::Png) => "png",
            Some(image::ImageFormat::Jpeg) => "jpg",
            Some(image::ImageFormat::WebP) => "webp",
            _ => {
                return Err(invalid(
                    "invalid_input_image",
                    "The input image format is unsupported.",
                    "Use PNG, JPEG or WebP; renaming a file extension does not convert it.",
                )
                .into());
            }
        };
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader.decode().map_err(|_| {
            invalid(
                "invalid_input_image",
                "The input image is corrupt or exceeds decoding limits.",
                "Use a valid image no larger than 8192 pixels per dimension.",
            )
        })?;
        let (width, height) = (decoded.width(), decoded.height());
        drop(decoded);
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        Ok(Self {
            source,
            bytes,
            extension,
            sha256,
            width,
            height,
        })
    }
    pub fn metadata(&self, snapshot: &Path) -> Value {
        json!({"source_path":self.source,"snapshot_path":snapshot,"sha256":self.sha256,"width":self.width,"height":self.height})
    }
}
