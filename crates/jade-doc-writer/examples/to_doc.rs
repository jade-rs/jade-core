//! `cargo run -p jade-doc-writer --example to_doc -- in.docx out.doc`
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let docx = std::fs::read(&args[1]).expect("read the .docx");
    let flow = jade_docx_convert::read_docx(&docx).expect("a Word document");
    std::fs::write(
        &args[2],
        jade_doc_writer::write_doc(&flow).expect("written"),
    )
    .expect("write the .doc");
}
