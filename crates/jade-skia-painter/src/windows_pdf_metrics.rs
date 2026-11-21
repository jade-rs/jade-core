//! Source-font GDI metrics for physical Windows PDF text; no Word coordinates.
use super::{FontData, SkiaPainter, Typeface};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::Mutex;

type Handle = *mut c_void;
type Widths = [Option<f64>; 95];
const MAX_FONT_BYTES: usize = 64 * 1024 * 1024;
static FONT_LOCK: Mutex<()> = Mutex::new(());

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Abc {
    a: i32,
    b: u32,
    c: i32,
}

#[link(name = "gdi32")]
extern "system" {
    fn AddFontMemResourceEx(
        data: *const c_void,
        size: u32,
        reserved: Handle,
        count: *mut u32,
    ) -> Handle;
    fn RemoveFontMemResourceEx(font: Handle) -> i32;
    fn CreateCompatibleDC(dc: Handle) -> Handle;
    fn DeleteDC(dc: Handle) -> i32;
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strikeout: u32,
        charset: u32,
        output: u32,
        clip: u32,
        quality: u32,
        pitch: u32,
        face: *const u16,
    ) -> Handle;
    fn SelectObject(dc: Handle, object: Handle) -> Handle;
    fn DeleteObject(object: Handle) -> i32;
    fn GetFontData(dc: Handle, table: u32, offset: u32, data: *mut c_void, size: u32) -> u32;
    fn GetCharABCWidthsI(
        dc: Handle,
        first: u32,
        count: u32,
        glyphs: *const u16,
        widths: *mut Abc,
    ) -> i32;
}
struct MemoryFont(Handle);
impl Drop for MemoryFont {
    fn drop(&mut self) {
        unsafe {
            RemoveFontMemResourceEx(self.0);
        }
    }
}
struct Dc(Handle);
impl Drop for Dc {
    fn drop(&mut self) {
        unsafe {
            DeleteDC(self.0);
        }
    }
}
struct Font(Handle);
impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.0);
        }
    }
}
struct Selection {
    dc: Handle,
    previous: Handle,
}
impl Drop for Selection {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
        }
    }
}

pub(super) fn widths(face: &Typeface, text: &str) -> Option<Vec<f64>> {
    if text.is_empty() || !text.bytes().all(|c| (b' '..=b'~').contains(&c)) {
        return None;
    }
    let data = SkiaPainter::shaping_data(face)?;
    thread_local! {
        static CACHE: RefCell<VecDeque<(FontData, Option<Widths>)>> = const { RefCell::new(VecDeque::new()) };
    }
    let values = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(index) = cache
            .iter()
            .position(|(source, _)| Rc::ptr_eq(source, &data))
        {
            let entry = cache.remove(index)?;
            let result = entry.1;
            cache.push_front(entry);
            return result;
        }
        let result = measure(face, &data);
        cache.push_front((data, result));
        cache.truncate(32);
        result
    })?;
    text.bytes().map(|c| values[(c - b' ') as usize]).collect()
}

fn static_truetype(data: &[u8]) -> bool {
    if data.len() < 12 || data.len() > MAX_FONT_BYTES || data[..4] != [0, 1, 0, 0] {
        return false;
    }
    let count = u16::from_be_bytes([data[4], data[5]]) as usize;
    let Some(tables) = data.get(12..12 + count * 16) else {
        return false;
    };
    !tables.chunks_exact(16).any(|table| &table[..4] == b"fvar")
}

fn measure(face: &Typeface, data: &FontData) -> Option<Widths> {
    if data.1 != 0
        || !static_truetype(&data.0)
        || face.font_style() != skia_safe::FontStyle::normal()
    {
        return None;
    }
    let mut family: Vec<u16> = face.family_name().encode_utf16().collect();
    if family.is_empty() || family.len() >= 32 || family.contains(&0) {
        return None;
    }
    family.push(0);
    let _lock = FONT_LOCK.lock().ok()?;
    // Memory fonts are process-private. Keep selection and removal under one lock.
    // Compare every selected font byte before accepting any GDI width.
    unsafe {
        let mut count = 0;
        let resource = AddFontMemResourceEx(
            data.0.as_ptr().cast(),
            data.0.len() as u32,
            std::ptr::null_mut(),
            &mut count,
        );
        if resource.is_null() {
            return None;
        }
        let _resource = MemoryFont(resource);
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let dc = Dc(dc);
        let font = CreateFontW(-1000, 0, 0, 0, 400, 0, 0, 0, 0, 0, 0, 0, 0, family.as_ptr());
        if font.is_null() {
            return None;
        }
        let font = Font(font);
        let previous = SelectObject(dc.0, font.0);
        if previous.is_null() || previous as isize == -1 {
            return None;
        }
        let _selection = Selection { dc: dc.0, previous };
        let size = GetFontData(dc.0, 0, 0, std::ptr::null_mut(), 0);
        if size as usize != data.0.len() {
            return None;
        }
        let mut selected = vec![0_u8; size as usize];
        if GetFontData(dc.0, 0, 0, selected.as_mut_ptr().cast(), size) != size || selected != data.0
        {
            return None;
        }
        let glyphs: Vec<u16> = (32..127).map(|c| face.unichar_to_glyph(c)).collect();
        let mut abc = [Abc::default(); 95];
        if GetCharABCWidthsI(dc.0, 0, 95, glyphs.as_ptr(), abc.as_mut_ptr()) == 0 {
            return None;
        }
        Some(std::array::from_fn(|i| {
            let width = i64::from(abc[i].a) + i64::from(abc[i].b) + i64::from(abc[i].c);
            (glyphs[i] != 0 && (1..=1_000_000).contains(&width)).then_some(width as f64)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::static_truetype;
    #[test]
    fn rejects_collections_variations_and_truncated_font_directories() {
        assert!(!static_truetype(&[]));
        let mut bytes = vec![0; 28];
        bytes[1] = 1;
        bytes[5] = 1;
        assert!(static_truetype(&bytes));
        bytes[12..16].copy_from_slice(b"fvar");
        assert!(!static_truetype(&bytes));
        bytes[12..16].copy_from_slice(b"head");
        bytes[5] = 2;
        assert!(!static_truetype(&bytes));
        bytes[..4].copy_from_slice(b"ttcf");
        assert!(!static_truetype(&bytes));
    }
}
