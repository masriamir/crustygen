//! CLI tests for `crustygen-corpus` over a temp directory.

mod common;

#[test]
fn the_stored_zip_helper_produces_an_archive_crustywad_can_read() {
    let wad = common::binary_entrada_wad();
    let zip = common::stored_zip(&[("README.TXT", b"hi"), ("ENTRADA.WAD", &wad)]);
    let archive = crustywad::archive::Archive::from_bytes(zip).expect("opens");
    assert_eq!(archive.members().len(), 2);
    let member = archive.member("ENTRADA.WAD").expect("member");
    let parsed = archive.wad(member).expect("reads as a WAD");
    assert_eq!(parsed.map_groups().len(), 1);
}
