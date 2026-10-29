//! Previews of the images whose paths a pane shows: the pointer on such a path shows the picture under it. Decoded by the
//! Windows Imaging Component, which reads every format Windows does (PNG, JPEG, GIF, BMP, TIFF, WebP, icons), scaled by it
//! to the size the preview is drawn at.

use std::path::Path;

use windows::{
    Win32::{
        Foundation::GENERIC_READ,
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPRGBA, IWICBitmapFrameDecode, IWICImagingFactory, WICBitmapDitherTypeNone,
            WICBitmapInterpolationModeFant, WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand,
        },
        System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx},
    },
    core::HSTRING,
};

/// The file types a preview is shown for.
const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico"];

/// Whether the file at `path` is an image the preview shows, by its extension.
pub fn is_image(path: &Path) -> bool {
    path.extension().is_some_and(|ext| EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

fn factory() -> Option<IWICImagingFactory> {
    // SAFETY: COM on the thread that asks (a second call on a thread is harmless); the factory is released when dropped.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()
    }
}

fn first_frame(factory: &IWICImagingFactory, path: &Path) -> Option<IWICBitmapFrameDecode> {
    // SAFETY: a decoder of the file, read on demand; it and its frame are released when dropped.
    unsafe {
        let decoder = factory.CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        decoder.GetFrame(0).ok()
    }
}

/// The size of the image in pixels, read from its header without decoding it; none for a file Windows cannot read.
pub fn size(path: &Path) -> Option<(u32, u32)> {
    let frame = first_frame(&factory()?, path)?;
    let (mut w, mut h) = (0, 0);
    // SAFETY: two locals the call writes.
    unsafe { frame.GetSize(&mut w, &mut h).ok()? };
    (w > 0 && h > 0).then_some((w, h))
}

/// The image (its first frame) scaled to `w` by `h` pixels, as premultiplied RGBA.
// shortcut: decodes on the thread of the window, the first time the pointer is on the path (a few tens of milliseconds for
// a large screenshot); move it to a thread if a photo of many megapixels makes the pointer stutter.
pub fn pixels(path: &Path, w: u32, h: u32) -> Option<Vec<u8>> {
    let factory = factory()?;
    let frame = first_frame(&factory, path)?;
    // SAFETY: a scaler and a converter of the frame, released when dropped; the buffer holds the rows the stride asks.
    unsafe {
        let scaler = factory.CreateBitmapScaler().ok()?;
        scaler.Initialize(&frame, w, h, WICBitmapInterpolationModeFant).ok()?;
        let converter = factory.CreateFormatConverter().ok()?;
        converter.Initialize(&scaler, &GUID_WICPixelFormat32bppPRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        converter.CopyPixels(std::ptr::null(), w * 4, &mut rgba).ok()?;
        Some(rgba)
    }
}

/// The size in DIP a preview of an image of `w` by `h` pixels is drawn at: within `max_w` by `max_h`, its shape kept, and
/// never larger than the image is at the scale `scale` of the window.
pub fn fit(w: u32, h: u32, max_w: f32, max_h: f32, scale: f32) -> (f32, f32) {
    let natural = (w as f32 / scale, h as f32 / scale);
    let shrink = (max_w / natural.0).min(max_h / natural.1).min(1.0);
    ((natural.0 * shrink).max(1.0), (natural.1 * shrink).max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_is_known_by_its_extension_whatever_its_case() {
        assert!(is_image(Path::new(r"C:\shots\sheet.png")) && is_image(Path::new("a.JPEG")) && is_image(Path::new("x.webp")));
        assert!(!is_image(Path::new("notes.txt")) && !is_image(Path::new("png")));
    }

    #[test]
    fn a_preview_keeps_the_shape_and_never_grows_past_the_image() {
        assert_eq!(fit(2000, 1000, 320.0, 220.0, 1.0), (320.0, 160.0), "wide: as wide as the room");
        assert_eq!(fit(1000, 2000, 320.0, 220.0, 1.0), (110.0, 220.0), "tall: as tall as the room");
        assert_eq!(fit(100, 50, 320.0, 220.0, 2.0), (50.0, 25.0), "small: its own size at the window's scale");
    }

    #[test]
    fn windows_reads_the_size_and_the_pixels_of_a_png() {
        let file = std::env::temp_dir().join(format!("nekominal-preview-{}.png", std::process::id()));
        // Two pixels wide, one high: red, then a half-transparent blue.
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        }
        std::fs::write(&file, png).unwrap();
        assert_eq!(size(&file), Some((2, 1)));
        let rgba = pixels(&file, 2, 1).unwrap();
        assert_eq!(&rgba[..4], [255, 0, 0, 255]);
        assert_eq!(rgba[7], 128, "the alpha kept");
        assert!(rgba[6] <= 129, "and premultiplied");
        assert_eq!(size(Path::new(r"C:\no\such\file.png")), None);
        let _ = std::fs::remove_file(&file);
    }
}
