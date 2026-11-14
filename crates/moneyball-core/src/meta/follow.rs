//! The e2e build's person at the browser (auth-plan.md, "In the e2e build
//! the open-the-browser step fetches the dialog URL itself"): nobody is
//! there to click, so the app GETs Meta's dialog, follows its redirects to
//! the broker, and submits the form on the broker's page as that page's
//! script would. Compiled only with core's `e2e` feature, which only the
//! Mac app's e2e build turns on.

use std::result::Result;

/// `connect`'s opener in the e2e build: returns at once, as a browser
/// does, and follows the dialog on a thread of its own.
pub fn follow_dialog(url: &str) -> Result<(), String> {
    let url = url.to_string();
    std::thread::spawn(move || {
        if let Err(e) = follow(&url) {
            eprintln!("e2e: following Meta's dialog failed: {}", e);
        }
    });
    Ok(())
}

fn follow(url: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::new();
    let page = client
        .get(url)
        .send()
        .and_then(|r| r.text())
        .map_err(|e| e.without_url().to_string())?;
    let (action, fields) = form(&page).ok_or("the broker's page has no form")?;
    client
        .post(action)
        .form(&fields)
        .send()
        .map(drop)
        .map_err(|e| e.without_url().to_string())
}

/// The first form's action and its inputs' names and values.
fn form(page: &str) -> Option<(String, Vec<(String, String)>)> {
    let action = attr(page.split("<form").nth(1)?, "action")?;
    let fields = page
        .split("<input")
        .skip(1)
        .filter_map(|tag| {
            let tag = tag.split('>').next()?;
            Some((attr(tag, "name")?, attr(tag, "value").unwrap_or_default()))
        })
        .collect();
    Some((action, fields))
}

/// `name="..."` in `tag`, unescaped as the broker escapes (`&#NN;`).
fn attr(tag: &str, name: &str) -> Option<String> {
    let (_, rest) = tag.split_once(&format!(" {}=\"", name))?;
    let raw = rest.split('"').next()?;
    let escapes = [
        ("&#34;", "\""),
        ("&#39;", "'"),
        ("&#60;", "<"),
        ("&#62;", ">"),
    ];
    let plain = escapes
        .iter()
        .fold(raw.to_string(), |s, (e, c)| s.replace(e, c));
    Some(plain.replace("&#38;", "&"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_brokers_form_is_read_as_the_browser_submits_it() {
        let page = "<form method=\"post\" action=\"http://127.0.0.1:5000/meta/done\">\
            <input type=\"hidden\" name=\"error\" value=\"Can&#39;t &#38; won&#39;t\">\
            <input type=\"hidden\" name=\"state\" value=\"abc\"><button>Go</button></form>";
        let (action, fields) = super::form(page).unwrap();
        assert_eq!(action, "http://127.0.0.1:5000/meta/done");
        let want = [("error", "Can't & won't"), ("state", "abc")];
        let want: Vec<_> = want
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(fields, want);
    }
}
