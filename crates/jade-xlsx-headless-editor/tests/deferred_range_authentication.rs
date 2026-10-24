use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{HeadlessWorkbook, RangeEditRequirement, RangeUnlockStatus};
use jade_xlsx_reader::CellRange;
fn open() -> HeadlessWorkbook {
    let mut p = jade_opc::OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    p.replace("xl/worksheets/sheet1.xml",br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/><sheetProtection sheet="1"/><protectedRanges><protectedRange name="First" sqref="A1" password="83AF"/><protectedRange name="Second" sqref="A2" password="83AF"/><protectedRange name="Alternative" sqref="A1" password="83AF"/></protectedRanges></worksheet>"#.to_vec());
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
fn req(b: &HeadlessWorkbook, s: &str) -> Vec<RangeEditRequirement> {
    vec![RangeEditRequirement {
        sheet: b.workbook().sheets[0].name.clone(),
        ranges: vec![CellRange::parse(s).unwrap()],
    }]
}
#[test]
fn verified_passwords_are_deferred_until_the_complete_batch_commits() {
    let mut b = open();
    let sheet = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    let context = b.range_auth_context();
    let requirements = req(&b, "A1:A2");
    assert!(
        matches!(b.range_unlock_requirements(&requirements,&[]).unwrap(),RangeUnlockStatus::Prompt(ref c) if c.len()==3)
    );
    assert!(b
        .authenticate_editable_range(&sheet, "First", "wrong")
        .is_err());
    assert_eq!(b.range_auth_context(), context);
    let first = b
        .authenticate_editable_range(&sheet, "First", "password")
        .unwrap();
    assert!(!b.editable_range_unlocked(&sheet, "First").unwrap());
    assert_eq!(b.bytes(), before);
    assert!(!b.can_undo());
    assert!(
        matches!(b.range_unlock_requirements(&requirements,std::slice::from_ref(&first)).unwrap(),RangeUnlockStatus::Prompt(ref c) if c.len()==1 && c[0].definition.name=="Second")
    );
    let second = b
        .authenticate_editable_range(&sheet, "Second", "password")
        .unwrap();
    assert_eq!(
        b.range_unlock_requirements(&requirements, &[first.clone(), second.clone()])
            .unwrap(),
        RangeUnlockStatus::Ready
    );
    assert!(b.apply_range_authentications(&[first, second]).unwrap());
    assert_eq!(b.bytes(), before);
    assert!(!b.can_undo());
    assert_eq!(
        b.range_unlock_requirements(&requirements, &[]).unwrap(),
        RangeUnlockStatus::Ready
    );
}
#[test]
fn uncovered_cells_do_not_offer_partial_password_authorization() {
    let b = open();
    assert_eq!(
        b.range_unlock_requirements(&req(&b, "A1:A3"), &[]).unwrap(),
        RangeUnlockStatus::Blocked
    );
}
#[test]
fn unrelated_workbooks_writes_history_and_relocking_invalidate_pending_proofs() {
    let mut b = open();
    let sheet = b.workbook().sheets[0].name.clone();
    let token = b
        .authenticate_editable_range(&sheet, "First", "password")
        .unwrap();
    let mut other = open();
    assert!(other
        .apply_range_authentications(std::slice::from_ref(&token))
        .is_err());
    b.unprotect_sheet(&sheet, "").unwrap();
    b.undo().unwrap();
    assert!(b.apply_range_authentications(&[token]).is_err());
    assert!(!b.editable_range_unlocked(&sheet, "First").unwrap());
    b.unlock_editable_range(&sheet, "First", "password")
        .unwrap();
    let token = b.authenticate_editable_range(&sheet, "First", "").unwrap();
    b.lock_editable_ranges(&sheet).unwrap();
    assert!(b.apply_range_authentications(&[token]).is_err());
}
#[test]
fn batch_validation_is_atomic_and_checkpoints_restore_grant_context() {
    let mut b = open();
    let sheet = b.workbook().sheets[0].name.clone();
    let first = b
        .authenticate_editable_range(&sheet, "First", "password")
        .unwrap();
    let other = open();
    let second = other
        .authenticate_editable_range(&sheet, "Second", "password")
        .unwrap();
    assert!(b
        .apply_range_authentications(&[first.clone(), second])
        .is_err());
    assert!(!b.editable_range_unlocked(&sheet, "First").unwrap());
    let context = b.range_auth_context();
    let checkpoint = b.edit_checkpoint();
    b.apply_range_authentications(std::slice::from_ref(&first))
        .unwrap();
    b.set(&sheet, "A1", CellInput::Number(42.)).unwrap();
    assert!(b.restore_edit_checkpoint(checkpoint));
    assert_eq!(b.range_auth_context(), context);
    assert!(!b.editable_range_unlocked(&sheet, "First").unwrap());
    assert!(b.apply_range_authentications(&[first]).unwrap());
}
