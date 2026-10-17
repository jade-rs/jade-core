//! Synthetic oriented JPEG for browser image-import regression coverage.
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::{error::Error, fs, io::Cursor};
fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args().nth(1).ok_or("expected output JPEG path")?;
    let pixels = RgbImage::from_fn(64, 32, |x, _| {
        if x < 32 {
            Rgb([240, 20, 20])
        } else {
            Rgb([20, 20, 240])
        }
    });
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(pixels).write_to(&mut encoded, ImageFormat::Jpeg)?;
    let encoded = encoded.into_inner();
    let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut image = encoded[..2].to_vec();
    image.extend([0xff, 0xe1]);
    image.extend(((exif.len() + 2) as u16).to_be_bytes());
    image.extend(exif);
    image.extend(&encoded[2..]);
    fs::write(path, image)?;
    Ok(())
}
