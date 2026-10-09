//! `moneyball schedule`: the morning refresh from the shell. Alone it says
//! what is scheduled; `--at HH:MM` installs (the flag is the consent: it
//! names the time), `--off` removes, `--print` shows the plist and writes
//! nothing, for anyone who installs it themselves.

use anyhow::Result;

use moneyball_core::schedule::{self, DEFAULT_AT};
use moneyball_core::AppConfig;

pub(crate) fn run(cfg: &AppConfig, at: Option<&str>, off: bool, print: bool) -> Result<()> {
    if off {
        println!(
            "{}",
            if schedule::remove(cfg)? {
                "schedule removed: refreshes run when you ask for one"
            } else {
                "no schedule to remove"
            }
        );
        return Ok(());
    }
    let time = match at {
        Some(t) => schedule::parse_at(t)?,
        None => schedule::installed(cfg).unwrap_or(DEFAULT_AT),
    };
    let plan = schedule::plan(cfg, &std::env::current_exe()?, time)?;
    if print {
        print!("{}", plan.plist_xml());
        return Ok(());
    }
    if at.is_none() {
        match schedule::installed(cfg) {
            Some(_) => describe("scheduled", &plan),
            None => println!(
                "no schedule: a refresh runs when you ask for one.\n\
                 moneyball schedule --at 07:30 refreshes every morning (--print shows the plist first)"
            ),
        }
        return Ok(());
    }
    schedule::install(&plan)?;
    describe("installed", &plan);
    Ok(())
}

fn describe(what: &str, plan: &schedule::Plan) {
    let next = schedule::next_from_now((plan.hour, plan.minute));
    println!(
        "{}: refresh every day at {:02}:{:02} (next {})",
        what,
        plan.hour,
        plan.minute,
        next.format("%a %d %b %H:%M")
    );
    println!("writes   {}", plan.plist.display());
    println!("runs     {}", plan.command());
    println!("logs     {}", plan.log.display());
    println!("remove   moneyball schedule --off");
}
