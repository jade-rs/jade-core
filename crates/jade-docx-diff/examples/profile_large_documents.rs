use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use jade_docx_diff::{DiffOptions, Limits, diff_documents, load_document};
use zip::write::SimpleFileOptions;

fn main() -> Result<()> {
    let mut rows = 50_000usize;
    let mut output_dir = std::env::temp_dir().join("jade-docx-diff-large-profile");
    let mut compare_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rows" => rows = args.next().context("--rows needs a value")?.parse()?,
            "--output-dir" => {
                output_dir = PathBuf::from(args.next().context("--output-dir needs a value")?)
            }
            "--compare-only" => compare_only = true,
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }

    std::fs::create_dir_all(&output_dir)?;
    let old_path = output_dir.join("large-old.docx");
    let new_path = output_dir.join("large-new.docx");
    if !compare_only || !old_path.exists() || !new_path.exists() {
        write_document(&old_path, rows, false)?;
        write_document(&new_path, rows, true)?;
    }

    let limits = Limits::default();
    let started = Instant::now();
    let old = load_document(&old_path, &limits)?;
    let old_loaded = Instant::now();
    let new = load_document(&new_path, &limits)?;
    let loaded = Instant::now();
    let diff = diff_documents(
        &old_path.display().to_string(),
        &new_path.display().to_string(),
        &old,
        &new,
        &DiffOptions::default(),
    );
    let finished = Instant::now();

    println!(
        "rows={rows} old_load_s={:.3} new_load_s={:.3} diff_s={:.3} total_s={:.3} paragraphs_added={} paragraphs_changed={}",
        (old_loaded - started).as_secs_f64(),
        (loaded - old_loaded).as_secs_f64(),
        (finished - loaded).as_secs_f64(),
        (finished - started).as_secs_f64(),
        diff.summary.paragraphs_added,
        diff.summary.paragraphs_changed,
    );
    anyhow::ensure!(diff.summary.paragraphs_added == 1);
    anyhow::ensure!(diff.summary.paragraphs_changed == 1);
    Ok(())
}

fn write_document(path: &Path, rows: usize, changed: bool) -> Result<()> {
    let file = File::create(path)?;
    let mut archive = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    archive.start_file("[Content_Types].xml", options)?;
    archive.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
    )?;
    archive.start_file("_rels/.rels", options)?;
    archive.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#,
    )?;
    archive.start_file("word/document.xml", options)?;
    archive.write_all(
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body>"#,
    )?;
    for row in 0..rows {
        if changed && row == rows / 2 {
            write_paragraph(&mut archive, 0xF000_0000, "追加された条項です。")?;
        }
        let text = if changed && row == rows / 3 {
            format!("第{row}条 この条項だけ改定しました。")
        } else {
            format!("第{row}条 この条項の本文です。")
        };
        write_paragraph(&mut archive, row as u32, &text)?;
    }
    archive.write_all(b"</w:body></w:document>")?;
    archive.finish()?;
    Ok(())
}

fn write_paragraph(archive: &mut zip::ZipWriter<File>, id: u32, text: &str) -> Result<()> {
    write!(
        archive,
        r#"<w:p w14:paraId="{id:08X}"><w:r><w:t>{text}</w:t></w:r></w:p>"#
    )?;
    Ok(())
}
