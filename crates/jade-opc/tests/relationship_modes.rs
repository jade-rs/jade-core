use jade_opc::parse_relationships;
#[test]
fn external_relationships_retain_their_target_mode() {
    let relationships=parse_relationships(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="embedded" Type="image" Target="../media/image.png"/><Relationship Id="linked" Type="image" Target="https://example.invalid/image.png" TargetMode="External"/></Relationships>"#).unwrap();
    assert!(!relationships.relationship("embedded").unwrap().external);
    assert!(relationships.relationship("linked").unwrap().external);
    assert_eq!(
        relationships.relationship("linked").unwrap().target,
        "https://example.invalid/image.png"
    );
}
