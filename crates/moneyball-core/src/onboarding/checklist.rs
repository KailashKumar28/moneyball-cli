//! The setup checklist above Today's report (plan Step 4): what is done
//! and what is left for one client, each item ticked from what is on disk
//! and worded here, so every front-end says the same.

use serde::Serialize;

use crate::config::AppConfig;
use crate::connections::{self, Group, Link};
use crate::text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    Meta,
    Accounts,
    Goals,
    Crm,
    Schedule,
}

/// One line of the checklist: its line says what is done, or what to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub check: Check,
    pub done: bool,
    pub line: String,
}

/// What the checklist reads, apart from its words.
struct Facts {
    token: bool,
    products: usize,
    /// Products without a goal.
    no_goal: usize,
    /// The connected CRM's name.
    crm: Option<String>,
    /// The morning refresh's time.
    schedule: Option<(u32, u32)>,
}

/// The checklist for `cfg`'s client, in order.
pub fn checklist(cfg: &AppConfig) -> Vec<Item> {
    let products = cfg.workspace.as_ref().map_or(&[][..], |w| &w.products);
    let goals = cfg.workspace.as_ref().map(|w| &w.goals);
    let crm = connections::status_all(cfg)
        .into_iter()
        .find(|s| s.group == Group::Leads && s.link != Link::Missing)
        .map(|s| s.name);
    items(&Facts {
        token: crate::secrets::load_meta_token().is_some(),
        products: products.len(),
        no_goal: products
            .iter()
            .filter(|p| !goals.is_some_and(|g| g.contains_key(&p.name)))
            .count(),
        crm,
        schedule: crate::schedule::installed(cfg),
    })
}

fn items(f: &Facts) -> Vec<Item> {
    // `done` says what is there; `todo` what to do.
    let item = |check, done: Option<String>, todo: &str| Item {
        check,
        done: done.is_some(),
        line: done.unwrap_or_else(|| todo.into()),
    };
    let products = text::count(f.products as u64, "product", "products");
    let (meta, accounts) = (f.token, f.products > 0);
    let goals = accounts && f.no_goal == 0;
    vec![
        item(
            Check::Meta,
            meta.then(|| "Meta connected".into()),
            "Connect Meta: one read-only token for every client",
        ),
        item(
            Check::Accounts,
            accounts.then(|| format!("Ad accounts chosen: {}", products)),
            "Choose the client's ad accounts",
        ),
        item(
            Check::Goals,
            goals.then(|| "Goals set for every product".into()),
            "Set goals: the qualified leads a day you aim for, per product",
        ),
        item(
            Check::Crm,
            f.crm
                .as_ref()
                .map(|name| format!("CRM connected: {}", name)),
            "Connect the CRM: it turns leads into qualified leads and verdicts",
        ),
        item(
            Check::Schedule,
            f.schedule
                .map(|(h, m)| format!("Morning refresh on at {:02}:{:02}", h, m)),
            "Turn on the morning refresh: yesterday's numbers every morning",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(f: &Facts) -> Vec<(Check, bool, String)> {
        items(f)
            .into_iter()
            .map(|i| (i.check, i.done, i.line))
            .collect()
    }

    #[test]
    fn nothing_done_says_what_to_do() {
        let got = items(&Facts {
            token: false,
            products: 0,
            no_goal: 0,
            crm: None,
            schedule: None,
        });
        let checks: Vec<Check> = got.iter().map(|i| i.check).collect();
        use Check::*;
        assert_eq!(checks, [Meta, Accounts, Goals, Crm, Schedule]);
        assert!(got.iter().all(|i| !i.done), "{:?}", got);
        assert_eq!(
            got[2].line,
            "Set goals: the qualified leads a day you aim for, per product"
        );
    }

    #[test]
    fn done_items_say_what_is_there() {
        let got = lines(&Facts {
            token: true,
            products: 4,
            no_goal: 0,
            crm: Some("LeadZump".into()),
            schedule: Some((7, 30)),
        });
        let want = [
            (Check::Meta, true, "Meta connected"),
            (Check::Accounts, true, "Ad accounts chosen: 4 products"),
            (Check::Goals, true, "Goals set for every product"),
            (Check::Crm, true, "CRM connected: LeadZump"),
            (Check::Schedule, true, "Morning refresh on at 07:30"),
        ];
        let want: Vec<_> = want
            .iter()
            .map(|(c, d, l)| (*c, *d, l.to_string()))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_product_without_a_goal_leaves_goals_open() {
        let got = items(&Facts {
            token: true,
            products: 1,
            no_goal: 1,
            crm: None,
            schedule: None,
        });
        assert_eq!(got[1].line, "Ad accounts chosen: 1 product");
        assert!(!got[2].done);
    }
}
