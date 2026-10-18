//! Dark paper: a document's colours re-mapped for reading on a dark page.
//!
//! A white page under a dark app glares, so the screen can show the page
//! dark instead — Word's dark page mode. The document is not changed: the
//! mapping is applied as colours reach the painter, and only by a painter told
//! to, so printing and export keep the colours the author chose.
//!
//! The mapping turns lightness over in OKLab and leaves hue alone: white paper
//! becomes a dark grey, black ink a light one, and a dark red heading a light
//! red. Turning the RGB channels over instead would send that red to cyan.
//! Pictures are not colours and never come through here; inverting a photo
//! makes it a negative.

/// OKLab lightness white paper is mapped to. A dark grey rather than black,
/// so the page still reads as a sheet against the app's darker canvas.
const PAPER_LIGHTNESS: f32 = 0.235;
/// OKLab lightness black ink is mapped to: the app's own dark-mode text,
/// rather than pure white, which vibrates on a dark ground.
const INK_LIGHTNESS: f32 = 0.905;

/// What white paper becomes, as an sRGB triple in `0.0..=1.0`.
pub fn dark_paper() -> [f32; 3] {
    to_dark_paper([1.0, 1.0, 1.0])
}

/// Maps an sRGB colour (`0.0..=1.0` per channel) onto the dark page.
///
/// Lightness runs from [`PAPER_LIGHTNESS`] (for white) to [`INK_LIGHTNESS`]
/// (for black); hue is kept, and chroma is reduced only as far as the result
/// needs to stay displayable — a saturated yellow highlight has no dark
/// counterpart as saturated as itself.
pub fn to_dark_paper(rgb: [f32; 3]) -> [f32; 3] {
    let [lightness, a, b] =
        linear_to_oklab(rgb.map(|channel| srgb_to_linear(channel.clamp(0.0, 1.0))));
    let lightness = INK_LIGHTNESS - lightness.clamp(0.0, 1.0) * (INK_LIGHTNESS - PAPER_LIGHTNESS);
    let fits = |chroma: f32| in_gamut(oklab_to_linear([lightness, a * chroma, b * chroma]));
    let chroma = if fits(1.0) {
        1.0
    } else {
        // Grey always fits, so the largest chroma that does lies in 0..1.
        let (mut low, mut high) = (0.0_f32, 1.0_f32);
        for _ in 0..16 {
            let mid = (low + high) / 2.0;
            if fits(mid) {
                low = mid;
            } else {
                high = mid;
            }
        }
        low
    };
    oklab_to_linear([lightness, a * chroma, b * chroma])
        .map(|channel| linear_to_srgb(channel.clamp(0.0, 1.0)))
}

/// [`to_dark_paper`] for 8-bit channels.
pub fn to_dark_paper_u8(rgb: [u8; 3]) -> [u8; 3] {
    to_dark_paper(rgb.map(|channel| f32::from(channel) / 255.0))
        .map(|channel| (channel * 255.0).round() as u8)
}

fn in_gamut(linear: [f32; 3]) -> bool {
    const EPSILON: f32 = 1e-4;
    linear
        .iter()
        .all(|channel| (-EPSILON..=1.0 + EPSILON).contains(channel))
}

fn srgb_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(channel: f32) -> f32 {
    if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

// The matrices are Björn Ottosson's, from the OKLab definition.

fn linear_to_oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn oklab_to_linear([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lightness(rgb: [u8; 3]) -> f32 {
        linear_to_oklab(rgb.map(|channel| srgb_to_linear(f32::from(channel) / 255.0)))[0]
    }

    #[test]
    fn white_paper_turns_dark_grey_and_black_ink_light_grey() {
        let paper = to_dark_paper_u8([255, 255, 255]);
        let ink = to_dark_paper_u8([0, 0, 0]);
        assert!(
            (lightness(paper) - PAPER_LIGHTNESS).abs() < 0.01,
            "{paper:?}"
        );
        assert!((lightness(ink) - INK_LIGHTNESS).abs() < 0.01, "{ink:?}");
        for grey in [paper, ink] {
            assert!(
                grey[0].abs_diff(grey[1]) <= 1 && grey[1].abs_diff(grey[2]) <= 1,
                "{grey:?}"
            );
        }
    }

    #[test]
    fn lightness_turns_over_and_keeps_its_order() {
        let greys = [0u8, 64, 128, 192, 255].map(|v| lightness(to_dark_paper_u8([v, v, v])));
        assert!(greys.windows(2).all(|pair| pair[0] > pair[1]), "{greys:?}");
    }

    #[test]
    fn a_red_stays_red_rather_than_turning_cyan() {
        let [r, g, b] = to_dark_paper_u8([0xc0, 0x00, 0x00]);
        assert!(r > g && r > b, "{:?}", [r, g, b]);
        assert!(lightness([r, g, b]) > lightness([0xc0, 0x00, 0x00]));
    }

    #[test]
    fn a_saturated_highlight_is_brought_into_gamut() {
        let [r, g, b] = to_dark_paper_u8([0xff, 0xff, 0x00]);
        assert!(b < r && b < g, "still yellowish: {:?}", [r, g, b]);
        assert!(
            lightness([r, g, b]) < 0.4,
            "a dark highlight: {:?}",
            [r, g, b]
        );
    }
}
