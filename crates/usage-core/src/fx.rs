//! Exchange rates from the Czech National Bank (ČNB) daily fixing.
//!
//! Format of `denni_kurz.txt`:
//! ```text
//! 27.10.2026 #207
//! země|měna|množství|kód|kurz
//! EMU|euro|1|EUR|24,340
//! Japonsko|jen|100|JPY|13,835
//! USA|dolar|1|USD|21,834
//! ```
//! `kurz` is CZK per `množství` units, with a decimal comma.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::model::{Currency, PlanCurrency};
use crate::{diag::Diag, Clock, CoreMsg, Update};

pub(crate) const CNB_URL: &str =
    "https://www.cnb.cz/cs/financni-trhy/devizovy-trh/kurzy-devizoveho-trhu/kurzy-devizoveho-trhu/denni_kurz.txt";

/// Fallback when no rate was ever fetched: ČNB fixing of 27.10.2026 (#207),
/// downloaded 2026-10-28.
pub(crate) const FALLBACK_CZK_PER_USD: f64 = 21.834;
pub(crate) const FALLBACK_CZK_PER_EUR: f64 = 24.340;
pub(crate) const FALLBACK_DATE: &str = "2026-10-27";

/// Retry a failed fetch after this many seconds.
const RETRY_SECS: i64 = 3600;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FxDay {
    /// ČNB fixing date, `YYYY-MM-DD`.
    pub date: String,
    pub czk_per_usd: f64,
    pub czk_per_eur: f64,
}

impl FxDay {
    pub(crate) fn fallback() -> FxDay {
        FxDay {
            date: FALLBACK_DATE.to_string(),
            czk_per_usd: FALLBACK_CZK_PER_USD,
            czk_per_eur: FALLBACK_CZK_PER_EUR,
        }
    }

    /// EUR per USD = (CZK/USD) ÷ (CZK/EUR).
    pub(crate) fn eur_per_usd(&self) -> f64 {
        self.czk_per_usd / self.czk_per_eur
    }

    /// Multiply a USD amount by this to get the display currency.
    pub(crate) fn usd_to(&self, currency: Currency) -> f64 {
        match currency {
            Currency::Eur => self.eur_per_usd(),
            Currency::Czk => self.czk_per_usd,
        }
    }

    /// Convert an amount in a plan currency to the display currency.
    pub(crate) fn convert(&self, amount: f64, from: PlanCurrency, to: Currency) -> f64 {
        let czk = match from {
            PlanCurrency::Czk => amount,
            PlanCurrency::Eur => amount * self.czk_per_eur,
            PlanCurrency::Usd => amount * self.czk_per_usd,
        };
        match to {
            Currency::Czk => czk,
            Currency::Eur => czk / self.czk_per_eur,
        }
    }
}

pub(crate) fn parse_cnb(text: &str) -> Option<FxDay> {
    let mut lines = text.lines();
    let header = lines.next()?.trim();
    let date_part = header.split_whitespace().next()?;
    let mut dmy = date_part.split('.');
    let (d, m, y) = (dmy.next()?, dmy.next()?, dmy.next()?);
    let date = format!(
        "{:04}-{:02}-{:02}",
        y.parse::<u32>().ok()?,
        m.parse::<u32>().ok()?,
        d.parse::<u32>().ok()?
    );
    let mut usd = None;
    let mut eur = None;
    for line in lines {
        let cols: Vec<&str> = line.split('|').collect();
        if cols.len() != 5 {
            continue;
        }
        let Ok(amount) = cols[2].trim().parse::<f64>() else { continue };
        let Ok(rate) = cols[4].trim().replace(',', ".").parse::<f64>() else { continue };
        if amount <= 0.0 || rate <= 0.0 {
            continue;
        }
        match cols[3].trim() {
            "USD" => usd = Some(rate / amount),
            "EUR" => eur = Some(rate / amount),
            _ => {}
        }
    }
    Some(FxDay {
        date,
        czk_per_usd: usd?,
        czk_per_eur: eur?,
    })
}

/// Background fetcher: at most one successful fetch per Prague calendar day,
/// retry after an hour on failure. Ends when the stop channel disconnects.
pub(crate) fn spawn_fetcher(
    tx: Sender<CoreMsg>,
    stop: Receiver<()>,
    mut last_fetched_on: Option<String>,
    clock: Clock,
    diag: Diag,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("usage-fx".into())
        .spawn(move || {
            let mut last_attempt: Option<i64> = None;
            loop {
                let now = clock();
                let today = crate::score::prague_date_string(now);
                let due = last_fetched_on.as_deref() != Some(today.as_str())
                    && last_attempt.is_none_or(|t| now - t >= RETRY_SECS);
                if due {
                    last_attempt = Some(now);
                    match crate::http_get_text(CNB_URL).map(|t| parse_cnb(&t)) {
                        Ok(Some(day)) => {
                            last_fetched_on = Some(today.clone());
                            if tx
                                .send(CoreMsg::Update(Update::Fx { day, fetched_on: today }))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Ok(None) => diag.log(now, "fx", "ČNB file did not parse (USD/EUR missing)"),
                        Err(e) => diag.log(now, "fx", &format!("ČNB fetch failed: {e}")),
                    }
                }
                match stop.recv_timeout(Duration::from_secs(5)) {
                    Err(RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
            }
        })
        .expect("spawn fx thread")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "27.10.2026 #207\nzemě|měna|množství|kód|kurz\nAustrálie|dolar|1|AUD|15,354\nEMU|euro|1|EUR|24,340\nJaponsko|jen|100|JPY|13,835\nUSA|dolar|1|USD|21,834\n";

    #[test]
    fn parses_cnb_daily_file() {
        let day = parse_cnb(SAMPLE).unwrap();
        assert_eq!(day.date, "2026-10-27");
        assert!((day.czk_per_usd - 21.834).abs() < 1e-9);
        assert!((day.czk_per_eur - 24.340).abs() < 1e-9);
        assert!((day.eur_per_usd() - 21.834 / 24.340).abs() < 1e-12);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_cnb("").is_none());
        assert!(parse_cnb("<html>nope</html>").is_none());
        assert!(parse_cnb("27.10.2026 #207\nzemě|měna|množství|kód|kurz\nEMU|euro|1|EUR|24,340\n").is_none());
    }

    #[test]
    fn converts_plan_currencies() {
        let fx = FxDay::fallback();
        let eur = fx.convert(100.0, PlanCurrency::Usd, Currency::Eur);
        assert!((eur - 100.0 * 21.834 / 24.340).abs() < 1e-9);
        assert!((fx.convert(24.34, PlanCurrency::Czk, Currency::Eur) - 1.0).abs() < 1e-9);
        assert!((fx.convert(1.0, PlanCurrency::Eur, Currency::Czk) - 24.34).abs() < 1e-9);
        assert!((fx.usd_to(Currency::Czk) - 21.834).abs() < 1e-9);
    }
}
