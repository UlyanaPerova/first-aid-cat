//! Данные виджета: пять строк как в концептах (Сеть, VPN, ИИ, ЦП, Диск), итог, время, уровни нагрузки.

/// Данные для полей шаблона: строки 0 Сеть, 1 VPN, 2 ИИ, 3 ЦП, 4 Диск.
#[derive(Clone)]
pub struct TplData {
    pub rows: [(String, String); 5],
    pub total: (String, String),
    pub clock: String,
    pub cpu: Option<f64>,
    pub disk_used: Option<f64>,
    /// 0 ok, 1 warn, 2 bad
    pub state: usize,
}
