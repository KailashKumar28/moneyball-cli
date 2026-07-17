//! moneyball ASCII text logo (`figlet -f standard MONEYBALL`, 5 rows).

pub const LOGO: &str = "\
 __  __  ___  _   _ _______   ______    _    _     _
|  \\/  |/ _ \\| \\ | | ____\\ \\ / / __ )  / \\  | |   | |
| |\\/| | | | |  \\| |  _|  \\ V /|  _ \\ / _ \\ | |   | |
| |  | | |_| | |\\  | |___  | | | |_) / ___ \\| |___| |___
|_|  |_|\\___/|_| \\_|_____| |_| |____/_/   \\_\\_____|_____|";

#[cfg(test)]
mod tests {
    #[test]
    fn logo_rows_align() {
        let rows: Vec<&str> = super::LOGO.lines().collect();
        assert_eq!(rows.len(), 5);
        // Every row fits an 80-col terminal with the render indent.
        assert!(rows.iter().all(|r| r.len() <= 76));
    }
}
