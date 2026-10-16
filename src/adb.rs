use anyhow::{bail, Context, Result};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

pub use crate::process::{hidden, output_timeout};
#[derive(Clone, Debug)]
pub struct Device {
    pub serial: String,
    pub state: String,
    pub model: String,
}
#[derive(Clone)]
pub struct Tools {
    pub dir: PathBuf,
    pub data: PathBuf,
}
impl Tools {
    pub fn find(data: PathBuf) -> Result<Self> {
        let exe = std::env::current_exe()?;
        let base = exe.parent().unwrap_or(Path::new("."));
        let candidates = std::iter::once(base.join("tools/native"));
        #[cfg(any(debug_assertions, test, feature = "qa"))]
        let candidates = candidates.chain(std::iter::once(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/native"),
        ));
        for dir in candidates {
            if dir.join("libpocketplay_device.so").is_file()
                && dir.join("pocketplay-launcher.jar").is_file()
                && dir.join("adb.exe").is_file()
            {
                return Ok(Self { dir, data });
            }
        }
        bail!("Native tools are missing. Keep the tools folder beside PocketPlay.exe.")
    }
    pub fn adb_command(&self, serial: &str, args: &[&str]) -> Command {
        let mut c = hidden(&self.dir.join("adb.exe"));
        c.current_dir(&self.dir);
        if !serial.is_empty() {
            c.args(["-s", serial]);
        }
        c.args(args);
        c
    }
    pub fn run(&self, serial: &str, args: &[&str], seconds: u64) -> Result<String> {
        Ok(String::from_utf8_lossy(&output_timeout(
            self.adb_command(serial, args),
            Duration::from_secs(seconds),
            None,
        )?)
        .into())
    }
    pub fn devices(&self, mode: &str) -> Result<Vec<Device>> {
        let out = self.run("", &["devices", "-l"], 15)?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let p: Vec<_> = l.split_whitespace().collect();
                if p.len() < 2
                    || !["device", "offline", "unauthorized"].contains(&p[1])
                    || p[0].starts_with("emulator-")
                {
                    return None;
                }
                let wifi = p[0].contains(':') || p[0].contains("._adb-tls-");
                if wifi != (mode == "Wi-Fi") {
                    return None;
                }
                let model = p
                    .iter()
                    .find_map(|s| s.strip_prefix("model:"))
                    .unwrap_or("Android")
                    .replace('_', " ");
                Some(Device {
                    serial: p[0].into(),
                    state: p[1].into(),
                    model,
                })
            })
            .collect())
    }
    pub fn resolve(&self, serial: &str, mode: &str) -> Result<String> {
        let d = self.devices(mode)?;
        let good: Vec<_> = d.iter().filter(|d| d.state == "device").collect();
        if !serial.is_empty() {
            if good.iter().any(|d| d.serial == serial) {
                return Ok(serial.into());
            }
            bail!("Selected device is not ready. Refresh and authorize debugging on the phone.");
        }
        if good.len() == 1 {
            return Ok(good[0].serial.clone());
        }
        bail!(
            "{}",
            if d.is_empty() {
                "No ADB device found. Enable debugging on the phone, reconnect and Refresh."
            } else if good.is_empty() {
                "Device is offline or unauthorized. Unlock the phone and accept the debugging prompt."
            } else {
                "Select a device before starting."
            }
        )
    }
    pub fn kill_server(&self) -> Result<()> {
        self.run("", &["kill-server"], 10)?;
        Ok(())
    }
    pub fn snapshot(&self, serial: &str) -> Result<Vec<u8>> {
        output_timeout(
            self.adb_command(serial, &["exec-out", "screencap", "-p"]),
            Duration::from_secs(15),
            None,
        )
    }
    pub fn pair(&self, address: &str, code: &str) -> Result<String> {
        let address = address.trim();
        endpoint(address)?;
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            bail!("Enter the six-digit pairing code.");
        }
        let out = output_timeout(
            self.adb_command("", &["pair", address]),
            Duration::from_secs(30),
            Some(&format!("{code}\n")),
        )?;
        let text = String::from_utf8_lossy(&out);
        if !text.to_lowercase().contains("successfully paired") {
            bail!(
                "Pairing failed: {}",
                text.trim().replace(code, "[redacted]")
            );
        }
        Ok("Paired. Connect using the address from the main Wireless debugging screen.".into())
    }
    pub fn connect(&self, address: &str) -> Result<String> {
        let address = address.trim();
        endpoint(address)?;
        let out = self.run("", &["connect", address], 20)?;
        if !out.contains("connected to") {
            bail!("{}", out.trim());
        }
        Ok(address.into())
    }
    pub fn discover(&self) -> Result<Vec<String>> {
        Ok(self
            .run("", &["mdns", "services"], 15)?
            .lines()
            .filter(|l| l.contains("_adb-tls-connect._tcp"))
            .filter_map(|l| l.split_whitespace().last().map(String::from))
            .collect())
    }
    pub fn log_spawn(&self, mut c: Command, name: &str) -> Result<Child> {
        fs::create_dir_all(&self.data)?;
        let file = File::create(self.data.join(name))?;
        c.stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone()?))
            .stderr(Stdio::from(file));
        Ok(c.spawn()?)
    }
    pub fn encoders(&self, serial: &str) -> Result<Vec<String>> {
        let nonce = crate::native::session_token()?;
        let remote = format!("/data/local/tmp/pocketplay-{nonce}");
        let result = (|| -> Result<Vec<String>> {
            for (file, suffix) in [
                ("pocketplay-launcher.jar", ".jar"),
                ("libpocketplay_device.so", ".so"),
            ] {
                self.run(
                    serial,
                    &[
                        "push",
                        &self.dir.join(file).to_string_lossy(),
                        &format!("{remote}{suffix}"),
                    ],
                    30,
                )?;
            }
            let text = self.run(
                serial,
                &[
                    "shell",
                    &format!("CLASSPATH={remote}.jar:/system/framework/services.jar"),
                    "app_process",
                    "/",
                    "com.pocketplay.server.Main",
                    &format!("{remote}.so"),
                    "--encoders",
                ],
                20,
            )?;
            Ok(text
                .lines()
                .filter(|l| l.contains(" | video/"))
                .map(str::to_owned)
                .collect())
        })();
        let _ = self.run(
            serial,
            &[
                "shell",
                "rm",
                "-f",
                &format!("{remote}.jar"),
                &format!("{remote}.so"),
            ],
            5,
        );
        result
    }
}
pub fn endpoint(address: &str) -> Result<()> {
    if address.len() > 256 {
        bail!("Device address is too long.");
    }
    let (host, port) = address
        .trim()
        .rsplit_once(':')
        .context("Enter IP:port, for example 192.168.1.20:37123.")?;
    let port: u16 = port.parse().context("Invalid port.")?;
    let host_valid = if let Some(ip) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        ip.parse::<std::net::Ipv6Addr>().is_ok()
    } else if host.parse::<std::net::Ipv4Addr>().is_ok() {
        true
    } else {
        let host = host.strip_suffix('.').unwrap_or(host);
        !host.is_empty()
            && host.len() <= 253
            && !host.chars().all(|c| c.is_ascii_digit() || c == '.')
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
    };
    if port == 0 || !host_valid {
        bail!("Invalid device address.");
    }
    Ok(())
}
pub fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_accepts_ip_and_dns_and_rejects_malformed_targets() {
        for address in ["192.168.1.20:37123", "phone.local:5555", "[::1]:5555"] {
            endpoint(address).unwrap();
        }
        for address in [
            "--help:5555",
            "-host:5555",
            "host-:5555",
            "[bad]:5555",
            "999.1.1.1:5555",
            "host:0",
            "host:65536",
            "a;b:5555",
        ] {
            assert!(
                endpoint(address).is_err(),
                "Accepted invalid endpoint: {address}"
            );
        }
    }
}
