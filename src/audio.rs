use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
};

pub const SINK: &str = "tuna_board_bus";
pub const SOURCE: &str = "tuna_board_mic";

#[derive(Clone)]
pub struct Device {
    pub name: String,
    pub description: String,
}

fn pactl(args: &[&str]) -> Result<String> {
    let o = Command::new("pactl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("pactl est nécessaire (PipeWire PulseAudio)")?;
    if !o.status.success() {
        bail!("Audio : {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

pub fn default_sink() -> Result<String> {
    pactl(&["get-default-sink"])
}
pub fn default_source() -> Result<String> {
    pactl(&["get-default-source"])
}
pub fn devices(kind: &str) -> Result<Vec<Device>> {
    let raw = pactl(&["-f", "json", "list", kind])?;
    let a: Vec<Value> = serde_json::from_str(&raw)?;
    Ok(a.into_iter()
        .filter_map(|v| {
            let name = v["name"].as_str()?.to_string();
            if name.ends_with(".monitor") || name.starts_with("tuna_board_") {
                return None;
            }
            Some(Device {
                description: v["description"].as_str().unwrap_or(&name).to_string(),
                name,
            })
        })
        .collect())
}

pub struct Audio {
    modules: Vec<u32>,
    players: Vec<Child>,
    pub connected: bool,
    sink: String,
    source: String,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            modules: Vec::new(),
            players: Vec::new(),
            connected: false,
            sink: SINK.into(),
            source: SOURCE.into(),
        }
    }
}

impl Audio {
    /// Reprendre notre graphe après une mise à jour sans couper les applications qui l’utilisent.
    pub fn resume_existing(&mut self) -> Result<Option<String>> {
        let raw = pactl(&["list", "short", "modules"])?;
        let modules: Vec<_> = raw
            .lines()
            .filter_map(|l| {
                let mut fields = l.splitn(3, '\t');
                Some((
                    fields.next()?.parse::<u32>().ok()?,
                    fields.next()?,
                    fields.next().unwrap_or_default(),
                ))
            })
            .collect();
        let find = |name: &str, argument: &str| {
            modules
                .iter()
                .find(|m| m.1 == name && m.2.split_whitespace().any(|w| w == argument))
        };
        let Some(sink) = find("module-null-sink", &format!("sink_name={}", self.sink)) else {
            return Ok(None);
        };
        let Some(source) = find(
            "module-remap-source",
            &format!("source_name={}", self.source),
        ) else {
            return Ok(None);
        };
        let mut ids = vec![sink.0, source.0];
        let mic = if let Some(m) = find("module-loopback", &format!("sink={}", self.sink)) {
            ids.push(m.0);
            m.2.split_whitespace()
                .find_map(|w| w.strip_prefix("source="))
                .map(str::to_owned)
        } else {
            None
        };
        self.modules = ids;
        self.connected = true;
        Ok(mic)
    }
    pub fn source_description(&self) -> String {
        let list = pactl(&["-f", "json", "list", "sources"])
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok());
        list.and_then(|v| v.into_iter().find(|v| v["name"] == self.source))
            .and_then(|v| v["description"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Tuna".into())
    }
    /// Espace de noms distinct pour des tests audio simultanés sans toucher au graphe de l’application.
    pub fn with_namespace(namespace: &str) -> Result<Self> {
        if namespace.is_empty()
            || !namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            bail!("Espace de noms audio incorrect");
        }
        Ok(Self {
            sink: format!("tuna_board_{namespace}_bus"),
            source: format!("tuna_board_{namespace}_mic"),
            modules: Vec::new(),
            players: Vec::new(),
            connected: false,
        })
    }
    pub fn sink_name(&self) -> &str {
        &self.sink
    }
    pub fn source_name(&self) -> &str {
        &self.source
    }
    fn load(&mut self, module: &str, args: &[String]) -> Result<()> {
        let mut v = vec!["load-module", module];
        v.extend(args.iter().map(String::as_str));
        let id: u32 = pactl(&v)?
            .parse()
            .context("PipeWire n’a pas renvoyé d’identifiant de module")?;
        self.modules.push(id);
        Ok(())
    }
    pub fn connect(&mut self, microphone: Option<&str>) -> Result<()> {
        if self.connected {
            return Ok(());
        }
        // Ne jamais réutiliser ni retirer les périphériques d’une autre instance.
        let sinks = pactl(&["list", "short", "sinks"])?;
        if sinks
            .lines()
            .any(|l| l.split_whitespace().nth(1) == Some(self.sink.as_str()))
        {
            bail!("Une autre instance utilise déjà le micro Tuna Board. Fermez-la avant de réessayer.");
        }
        let result = (|| {
            self.load(
                "module-null-sink",
                &[
                    format!("sink_name={}", self.sink),
                    "rate=48000".into(),
                    "channels=2".into(),
                    "sink_properties=\"device.description=\\\"Tuna Board - Sons\\\"\"".into(),
                ],
            )?;
            self.load(
                "module-remap-source",
                &[
                    format!("master={}.monitor", self.sink),
                    format!("source_name={}", self.source),
                    "source_properties=device.description=Tuna".into(),
                ],
            )?;
            if let Some(mic) = microphone {
                if mic == self.source || mic.ends_with(".monitor") {
                    bail!("Choisissez un micro physique pour éviter une boucle audio");
                }
                self.load(
                    "module-loopback",
                    &[
                        format!("source={mic}"),
                        format!("sink={}", self.sink),
                        "latency_msec=20".into(),
                        "source_dont_move=true".into(),
                        "sink_dont_move=true".into(),
                    ],
                )?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.disconnect();
        } else {
            self.connected = true;
        }
        result
    }
    pub fn disconnect(&mut self) {
        self.stop();
        for id in self.modules.drain(..).rev() {
            let _ = pactl(&["unload-module", &id.to_string()]);
        }
        self.connected = false;
    }
    fn spawn(&mut self, file: &Path, target: &str, volume: f64) -> Result<()> {
        let child = Command::new("pw-play")
            .args([
                "--target",
                target,
                "--volume",
                &format!("{:.3}", volume.clamp(0.0, 1.0)),
                "--latency",
                "30ms",
            ])
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("pw-play est nécessaire (PipeWire)")?;
        self.players.push(child);
        Ok(())
    }
    pub fn play(
        &mut self,
        file: &Path,
        headphones: &str,
        preview: bool,
        monitor: bool,
        volume: f64,
    ) -> Result<()> {
        self.stop();
        if preview || !self.connected {
            self.spawn(file, headphones, volume)?;
        } else {
            let sink = self.sink.clone();
            self.spawn(file, &sink, volume)?;
            if monitor {
                self.spawn(file, headphones, volume)?;
            }
        }
        Ok(())
    }
    pub fn reap(&mut self) -> Option<String> {
        let mut error = None;
        self.players.retain_mut(|p| match p.try_wait() {
            Ok(None) => true,
            Ok(Some(code)) => {
                if !code.success() {
                    let mut message = String::new();
                    if let Some(stderr) = p.stderr.take() {
                        let _ = stderr.take(4096).read_to_string(&mut message);
                    }
                    error = Some(if message.trim().is_empty() {
                        "Le périphérique audio a refusé la lecture".into()
                    } else {
                        message.trim().to_string()
                    });
                }
                false
            }
            Err(e) => {
                error = Some(e.to_string());
                false
            }
        });
        error
    }
    pub fn stop(&mut self) {
        for mut p in self.players.drain(..) {
            let _ = p.kill();
            let _ = p.wait();
        }
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        self.disconnect();
    }
}
