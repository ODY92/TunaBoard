use std::{fs, time::{SystemTime, UNIX_EPOCH}};
use tuna_board::{catalog, store};

#[test]
fn imported_sound_survives_a_cleared_cache_without_network() {
    let root = std::env::temp_dir().join(format!("tuna-import-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::env::set_var("XDG_DATA_HOME", root.join("data"));
    let imports = store::data_dir().join("imported");
    fs::create_dir_all(&imports).unwrap();
    let sound: catalog::Sound = serde_json::from_value(serde_json::json!({
        "id":"12e34e56-1234-1234-1234-123456789abc", "name":"Import local", "author":"Moi",
        "url":"", "audio":"offline:no-network", "duration_ms":100, "image":""
    })).unwrap();
    let samples = vec![0u8; 4800*2];
    let mut wav=Vec::new();
    wav.extend_from_slice(b"RIFF");wav.extend_from_slice(&(36u32+samples.len() as u32).to_le_bytes());wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());wav.extend_from_slice(&1u16.to_le_bytes());wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&48000u32.to_le_bytes());wav.extend_from_slice(&96000u32.to_le_bytes());wav.extend_from_slice(&2u16.to_le_bytes());wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());wav.extend_from_slice(&samples);
    let original=imports.join(format!("{}.audio",sound.id));fs::write(&original,&wav).unwrap();
    let cache=root.join("cache");
    let first=catalog::download(&sound,&cache).unwrap();assert!(fs::metadata(&first).unwrap().len()>44);
    fs::remove_dir_all(&cache).unwrap();
    let second=catalog::download(&sound,&cache).unwrap();assert_eq!(first,second);
    assert_eq!(&fs::read(second).unwrap()[..4],b"RIFF");assert_eq!(fs::read(original).unwrap(),wav);
    fs::remove_dir_all(root).unwrap();
}
