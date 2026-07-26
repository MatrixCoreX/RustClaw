use super::*;

#[test]
fn png_gif_and_jpeg_dimensions_are_machine_readable() {
    let mut png = vec![0; 24];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    png[16..20].copy_from_slice(&640u32.to_be_bytes());
    png[20..24].copy_from_slice(&480u32.to_be_bytes());
    assert_eq!(png_dimensions(&png), Some((640, 480)));

    let mut gif = b"GIF89a".to_vec();
    gif.extend_from_slice(&320u16.to_le_bytes());
    gif.extend_from_slice(&200u16.to_le_bytes());
    assert_eq!(gif_dimensions(&gif), Some((320, 200)));

    let jpeg = [
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x01, 0x2c, 0x02, 0x58, 0x03, 0x01, 0x11, 0x00,
        0x02, 0x11, 0x00, 0x03, 0x11, 0x00,
    ];
    assert_eq!(jpeg_dimensions(&jpeg), Some((600, 300)));
}

#[test]
fn directory_counts_are_stable_and_bounded() {
    let entries = vec![entry("b/two.png"), entry("a/one.png"), entry("a/three.png")];

    let (counts, truncated) = directory_counts(&entries, 1);

    assert_eq!(counts, vec![json!({"dir": "a", "count": 2})]);
    assert!(truncated);
}

fn entry(path: &str) -> ImageEntry {
    ImageEntry {
        path: path.to_string(),
        extension: "png".to_string(),
        mime_type: "image/png".to_string(),
        size_bytes: 1,
        modified_unix_ms: None,
        width: None,
        height: None,
    }
}
