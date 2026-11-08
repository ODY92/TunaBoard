use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::process::Command;

#[derive(Default)]
pub struct Hotkeys {
    keys: Vec<String>,
}
impl Hotkeys {
    pub fn enabled(&self) -> bool {
        !self.keys.is_empty()
    }
    pub fn enable(&mut self) -> Result<()> {
        if self.enabled() {
            return Ok(());
        }
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
            bail!("Les raccourcis globaux intégrés nécessitent Hyprland. F1–F8 restent disponibles dans la fenêtre.");
        }
        let output = Command::new("hyprctl").args(["binds", "-j"]).output()?;
        if !output.status.success() {
            bail!("Impossible de lire les raccourcis Hyprland");
        }
        let binds: Vec<Value> = serde_json::from_slice(&output.stdout)?;
        let mut keys: Vec<String> = (1..=8).map(|n| format!("F{n}")).collect();
        keys.push("Escape".into());
        for key in &keys {
            if binds.iter().any(|b| {
                b["modmask"].as_u64() == Some(72)
                    && b["key"]
                        .as_str()
                        .is_some_and(|k| k.eq_ignore_ascii_case(key))
            }) {
                bail!("Super + Alt + {key} est déjà utilisé. Aucun raccourci n’a été remplacé.");
            }
        }
        let exe = std::env::current_exe()?;
        let escaped = format!("'{}'", exe.to_string_lossy().replace('\'', "'\\''"));
        for (i, key) in keys.iter().enumerate() {
            let arg = if key == "Escape" {
                "--stop".into()
            } else {
                format!("--slot {}", i + 1)
            };
            let binding = format!("SUPER ALT,{key},exec,{escaped} {arg}");
            let o = Command::new("hyprctl")
                .args(["keyword", "bind", &binding])
                .output()
                .context("hyprctl indisponible")?;
            if !o.status.success()
                || !String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .eq_ignore_ascii_case("ok")
            {
                self.disable();
                bail!("Hyprland a refusé un raccourci. Les raccourcis de cette session ont été retirés.");
            }
            self.keys.push(key.clone());
        }
        Ok(())
    }
    pub fn disable(&mut self) {
        for key in self.keys.drain(..) {
            let _ = Command::new("hyprctl")
                .args(["keyword", "unbind", &format!("SUPER ALT,{key}")])
                .output();
        }
    }
}
impl Drop for Hotkeys {
    fn drop(&mut self) {
        self.disable();
    }
}
