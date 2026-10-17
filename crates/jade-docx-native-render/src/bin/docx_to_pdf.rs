//! SDK validation CLI using the live editor's shared renderer.
use jade_docx_native_render::{FaceSpec, NativeDocument};
use std::path::PathBuf;

fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        return Err(
            "usage: docx_to_pdf <input.docx> <output.pdf> [--font <family> <weight> <file>]..."
                .into(),
        );
    };
    let mut fonts = Vec::new();
    while let Some(flag) = args.next() {
        if flag != "--font" {
            return Err(format!("unexpected option: {}", flag.to_string_lossy()));
        }
        let family = args
            .next()
            .and_then(|value| value.into_string().ok())
            .filter(|family| !family.is_empty())
            .ok_or("--font requires a family")?;
        let weight = args
            .next()
            .and_then(|value| value.into_string().ok())
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|weight| *weight <= 1000)
            .ok_or("--font requires a weight from 0 to 1000")?;
        let path = PathBuf::from(args.next().ok_or("--font requires a font file")?);
        fonts.push((family, weight, path));
    }
    if fonts.is_empty() {
        let directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fonts/Noto_Sans_JP/static");
        for family in [
            "Noto Sans JP",
            "Times New Roman",
            "Yu Mincho",
            "MS Mincho",
            "MS Gothic",
            "serif",
        ] {
            for (weight, name) in [
                (400, "NotoSansJP-Regular.ttf"),
                (700, "NotoSansJP-Bold.ttf"),
            ] {
                fonts.push((family.to_string(), weight, directory.join(name)));
            }
        }
    }
    let faces = fonts
        .iter()
        .map(|(family, weight, path)| FaceSpec {
            family,
            weight: *weight,
            path,
        })
        .collect::<Vec<_>>();
    let bytes =
        std::fs::read(&input).map_err(|error| format!("{}: {error}", input.to_string_lossy()))?;
    let editor = NativeDocument::open(&bytes, 816.0, 1056.0, &faces)?;
    let pdf = editor.export_pdf()?;
    std::fs::write(&output, pdf)
        .map_err(|error| format!("{}: {error}", output.to_string_lossy()))?;
    println!(
        "exported {} pages to {}",
        editor.page_count(),
        output.to_string_lossy()
    );
    Ok(())
}
