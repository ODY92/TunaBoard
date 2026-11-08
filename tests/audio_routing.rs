use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::Duration,
};
use tuna_board::audio::Audio;

// Ce test crée une sortie virtuelle temporaire et n’ouvre aucun micro physique.
#[test]
#[ignore = "nécessite une session PipeWire locale"]
fn generated_sound_reaches_virtual_microphone_and_cleanup_restores_graph() {
    let base = std::env::temp_dir().join(format!("tuna-board-test-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let tone = base.join("generated.wav");
    let capture = base.join("capture.wav");
    let frames = 48_000_u32;
    let bytes = frames * 4;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + bytes).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16_u32.to_le_bytes());
    wav.extend(1_u16.to_le_bytes());
    wav.extend(2_u16.to_le_bytes());
    wav.extend(48_000_u32.to_le_bytes());
    wav.extend(192_000_u32.to_le_bytes());
    wav.extend(4_u16.to_le_bytes());
    wav.extend(16_u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(bytes.to_le_bytes());
    for i in 0..frames {
        let x = ((i as f64 * 440.0 * std::f64::consts::TAU / 48_000.0).sin() * 5000.0) as i16;
        wav.extend(x.to_le_bytes());
        wav.extend(x.to_le_bytes());
    }
    std::fs::File::create(&tone)
        .unwrap()
        .write_all(&wav)
        .unwrap();
    let default_before = Command::new("pactl")
        .arg("get-default-source")
        .output()
        .unwrap()
        .stdout;
    let mut audio = Audio::with_namespace(&format!("test_{}", std::process::id())).unwrap();
    let source = audio.source_name().to_string();
    let sink = audio.sink_name().to_string();
    audio.connect(None).unwrap();
    assert_eq!(audio.source_description(), "Tuna");
    let recorder = Command::new("timeout")
        .args([
            "10",
            "ffmpeg",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "pulse",
            "-i",
            &source,
            "-t",
            "2",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&capture)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(250));
    audio.play(&tone, &sink, false, false, 0.5).unwrap();
    let result = recorder.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "capture {} : {}",
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );
    let mut captured = Vec::new();
    std::fs::File::open(&capture)
        .unwrap()
        .read_to_end(&mut captured)
        .unwrap();
    let mut offset = 12;
    let mut samples = &[][..];
    while offset + 8 <= captured.len() {
        let n = u32::from_le_bytes(captured[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if &captured[offset..offset + 4] == b"data" {
            samples = &captured[offset + 8..(offset + 8 + n).min(captured.len())];
            break;
        }
        offset += 8 + n + (n % 2);
    }
    let values: Vec<_> = samples
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
        .collect();
    let peak = values.iter().copied().max().unwrap_or(0);
    assert!(
        (2000..=3000).contains(&peak),
        "Le signal injecté à 50 % doit arriver sur le micro virtuel : pic={peak}"
    );
    assert!(
        values.iter().filter(|&&v| v > 1000).count() > 10_000,
        "La capture doit contenir le signal audio, pas seulement un en-tête WAV"
    );
    audio.disconnect();
    let sources = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&sources.stdout).contains(&source));
    let default_after = Command::new("pactl")
        .arg("get-default-source")
        .output()
        .unwrap()
        .stdout;
    assert_eq!(
        default_before, default_after,
        "Le micro par défaut du système doit être conservé"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
#[ignore = "nécessite une session PipeWire locale"]
fn update_can_resume_its_audio_graph_without_recreating_the_microphone() {
    let namespace = format!("resume_test_{}", std::process::id());
    let mut original = Audio::with_namespace(&namespace).unwrap();
    original.connect(None).unwrap();
    let source = original.source_name().to_string();
    let index = |name: &str| {
        let o = Command::new("pactl")
            .args(["-f", "json", "list", "sources"])
            .output()
            .unwrap();
        let sources: Vec<serde_json::Value> = serde_json::from_slice(&o.stdout).unwrap();
        sources.into_iter().find(|s| s["name"] == name).unwrap()["index"]
            .as_u64()
            .unwrap()
    };
    let before = index(&source);
    // Simuler une ancienne instance arrêtée pendant la mise à jour.
    std::mem::forget(original);
    let mut resumed = Audio::with_namespace(&namespace).unwrap();
    resumed.resume_existing().unwrap();
    assert!(resumed.connected);
    assert_eq!(
        index(&source),
        before,
        "Le consommateur doit garder le même périphérique"
    );
    resumed.disconnect();
    let list = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&list.stdout).contains(&source));
}
