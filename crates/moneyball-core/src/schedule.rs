//! The morning refresh (frame R4): one launchd agent per workspace that
//! runs `moneyball refresh --scheduled` at a set time. launchd runs a job
//! the Mac slept through when it wakes; one it was off for waits for the
//! next morning, whose 28-day pull catches the data up.
//!
//! The one write outside the workspace and ~/.moneyball/ (AGENTS.md):
//! ~/Library/LaunchAgents/ai.moneyball.refresh.<id>.plist, only after the
//! user confirms (/schedule's y, or `moneyball schedule --at`), removed by
//! `--off`. macOS only.

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{Duration, NaiveDateTime, NaiveTime};

use crate::config::{home_dir, AppConfig};
use crate::error::{Error, Result};

pub const DEFAULT_AT: (u32, u32) = (7, 30);

/// Everything an install writes and runs, shown to the user before it is.
#[derive(Debug, Clone)]
pub struct Plan {
    pub label: String,
    pub plist: PathBuf,
    pub exe: PathBuf,
    pub data_root: PathBuf,
    pub log: PathBuf,
    pub home: PathBuf,
    pub hour: u32,
    pub minute: u32,
}

impl Plan {
    /// The command the agent runs, as a user would type it.
    pub fn command(&self) -> String {
        format!(
            "moneyball --data-root {} refresh --scheduled",
            self.data_root.display()
        )
    }

    pub fn plist_xml(&self) -> String {
        let s = |p: &Path| xml(&p.display().to_string());
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>--data-root</string>
    <string>{root}</string>
    <string>refresh</string>
    <string>--scheduled</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HOME</key>
    <string>{home}</string>
  </dict>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Hour</key>
    <integer>{hour}</integer>
    <key>Minute</key>
    <integer>{minute}</integer>
  </dict>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
            label = xml(&self.label),
            exe = s(&self.exe),
            root = s(&self.data_root),
            home = s(&self.home),
            hour = self.hour,
            minute = self.minute,
            log = s(&self.log),
        )
    }
}

/// The plan for this workspace at `at` (hour, minute), run by `exe`.
pub fn plan(cfg: &AppConfig, exe: &Path, at: (u32, u32)) -> Result<Plan> {
    let label = label(cfg)?;
    let home = home_dir().ok_or_else(|| Error::Config("no home directory".into()))?;
    Ok(Plan {
        plist: home
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", label)),
        label,
        exe: exe.to_path_buf(),
        data_root: cfg.data_root.clone(),
        log: cfg.runs_dir().join("schedule.log"),
        home,
        hour: at.0,
        minute: at.1,
    })
}

/// The installed time for this workspace, read back from its plist.
pub fn installed(cfg: &AppConfig) -> Option<(u32, u32)> {
    let path = plan(cfg, Path::new(""), DEFAULT_AT).ok()?.plist;
    let xml = std::fs::read_to_string(path).ok()?;
    let int_after = |key: &str| -> Option<u32> {
        let (_, rest) = xml.split_once(&format!("<key>{}</key>", key))?;
        let (_, rest) = rest.split_once("<integer>")?;
        rest.split_once("</integer>")?.0.trim().parse().ok()
    };
    Some((int_after("Hour")?, int_after("Minute")?))
}

/// The day this workspace's schedule was set (its plist's date): missed
/// mornings before it were never scheduled.
pub fn since(cfg: &AppConfig) -> Option<chrono::NaiveDate> {
    let path = plan(cfg, Path::new(""), DEFAULT_AT).ok()?.plist;
    let t: chrono::DateTime<chrono::Local> = std::fs::metadata(path).ok()?.modified().ok()?.into();
    Some(t.date_naive())
}

/// Write the plist and (re)load it. A previous schedule for this
/// workspace is replaced, never doubled.
pub fn install(p: &Plan) -> Result<()> {
    if let Some(dir) = p.plist.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if let Some(dir) = p.log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = launchctl(&["bootout", &format!("{}/{}", domain()?, p.label)]);
    let tmp = p.plist.with_extension("plist.tmp");
    std::fs::write(&tmp, p.plist_xml())?;
    std::fs::rename(&tmp, &p.plist)?;
    launchctl(&["bootstrap", &domain()?, &p.plist.display().to_string()])
}

/// Unload and delete this workspace's schedule. False when there was none.
pub fn remove(cfg: &AppConfig) -> Result<bool> {
    let p = plan(cfg, Path::new(""), DEFAULT_AT)?;
    if !p.plist.is_file() {
        return Ok(false);
    }
    let _ = launchctl(&["bootout", &format!("{}/{}", domain()?, p.label)]);
    std::fs::remove_file(&p.plist)?;
    Ok(true)
}

/// The next time the schedule fires from now, local time.
pub fn next_from_now(at: (u32, u32)) -> NaiveDateTime {
    next_run(at, chrono::Local::now().naive_local())
}

/// The next time the schedule fires after `now`.
pub fn next_run(at: (u32, u32), now: NaiveDateTime) -> NaiveDateTime {
    let time = NaiveTime::from_hms_opt(at.0, at.1, 0).unwrap_or_default();
    let today = now.date().and_time(time);
    if today > now {
        today
    } else {
        today + Duration::days(1)
    }
}

/// "07:30" -> (7, 30).
pub fn parse_at(s: &str) -> Result<(u32, u32)> {
    let bad = || Error::Config(format!("the time must be HH:MM (24-hour), got \"{}\"", s));
    let (h, m) = s.trim().split_once(':').ok_or_else(bad)?;
    let (h, m): (u32, u32) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok((h, m))
}

fn label(cfg: &AppConfig) -> Result<String> {
    let id = cfg
        .workspace
        .as_ref()
        .and_then(|w| w.workspace_id.clone())
        .ok_or_else(|| Error::Config("no workspace configured - run /setup first".into()))?;
    let short: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    Ok(format!("ai.moneyball.refresh.{}", short))
}

/// launchd's per-user domain, "gui/501".
fn domain() -> Result<String> {
    let out = Command::new("id").arg("-u").output()?;
    Ok(format!(
        "gui/{}",
        String::from_utf8_lossy(&out.stdout).trim()
    ))
}

fn launchctl(args: &[&str]) -> Result<()> {
    let out = Command::new("launchctl").args(args).output()?;
    if out.status.success() {
        return Ok(());
    }
    Err(Error::Config(format!(
        "launchctl {} failed: {}",
        args.first().copied().unwrap_or_default(),
        String::from_utf8_lossy(&out.stderr).trim()
    )))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse_and_the_next_run_rolls_over() {
        assert_eq!(parse_at("07:30").unwrap(), (7, 30));
        assert!(parse_at("24:00").is_err() && parse_at("7.30").is_err());
        let at = |s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        assert_eq!(
            next_run((7, 30), at("2026-10-09 06:00")),
            at("2026-10-09 07:30")
        );
        assert_eq!(
            next_run((7, 30), at("2026-10-09 07:30")),
            at("2026-10-10 07:30")
        );
    }

    #[test]
    fn the_plist_runs_a_scheduled_refresh_and_reads_back() {
        let p = Plan {
            label: "ai.moneyball.refresh.6f3a91c2".into(),
            plist: PathBuf::from("/tmp/x.plist"),
            exe: PathBuf::from("/bin/moneyball"),
            data_root: PathBuf::from("/data/a&b"),
            log: PathBuf::from("/data/a&b/.moneyball/runs/schedule.log"),
            home: PathBuf::from("/Users/me"),
            hour: 7,
            minute: 5,
        };
        let x = p.plist_xml();
        assert!(x.contains("<string>--scheduled</string>"));
        assert!(x.contains("/data/a&amp;b") && !x.contains("a&b"));
        assert!(x.contains("<key>Minute</key>\n    <integer>5</integer>"));
    }
}
