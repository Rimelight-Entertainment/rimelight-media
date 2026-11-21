//! Camera RAW pictures, read with `rawler` (pure Rust). The file is developed to RGB with rawler's default develop; when
//! that fails, the largest JPEG preview inside the file is shown instead. Viewing only: saving goes through "Save as".

use std::path::Path;

use image::DynamicImage;

use crate::picture::Picture;

pub fn load(path: &Path) -> Result<Picture, String> {
    let developed = rawler::decode_file(path).map_err(|e| e.to_string()).and_then(|raw| {
        let intermediate = rawler::imgop::develop::RawDevelop::default()
            .develop_intermediate(&raw)
            .map_err(|e| e.to_string())?;
        intermediate.to_dynamic_image().ok_or_else(|| "the picture has no pixels".to_string())
    });
    match developed {
        Ok(img) => Ok(to_picture(img)),
        Err(develop_err) => preview(path).map(to_picture).map_err(|e| format!("{develop_err} ({e})")),
    }
}

/// The largest embedded image of the file: the full JPEG, else the preview, else the thumbnail.
fn preview(path: &Path) -> Result<DynamicImage, String> {
    let src = rawler::rawsource::RawSource::new(path).map_err(|e| e.to_string())?;
    let decoder = rawler::get_decoder(&src).map_err(|e| e.to_string())?;
    let params = rawler::decoders::RawDecodeParams::default();
    let candidates = [
        decoder.full_image(&src, &params).ok().flatten(),
        decoder.preview_image(&src, &params).ok().flatten(),
        decoder.thumbnail_image(&src, &params).ok().flatten(),
    ];
    candidates
        .into_iter()
        .flatten()
        .max_by_key(|img| u64::from(img.width()) * u64::from(img.height()))
        .ok_or_else(|| "no embedded picture".to_string())
}

fn to_picture(img: DynamicImage) -> Picture {
    let rgba = img.into_rgba8();
    Picture { w: rgba.width(), h: rgba.height(), rgba: rgba.into_raw() }
}
