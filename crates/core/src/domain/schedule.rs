//! Agendamento (RN-80 a RN-82). Convertido para `OnCalendar` do systemd.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    fn systemd(self) -> &'static str {
        match self {
            Self::Mon => "Mon",
            Self::Tue => "Tue",
            Self::Wed => "Wed",
            Self::Thu => "Thu",
            Self::Fri => "Fri",
            Self::Sat => "Sat",
            Self::Sun => "Sun",
        }
    }

    /// "Toda segunda", "Todo sábado"…
    fn every_label(self) -> &'static str {
        match self {
            Self::Mon => "Toda segunda",
            Self::Tue => "Toda terça",
            Self::Wed => "Toda quarta",
            Self::Thu => "Toda quinta",
            Self::Fri => "Toda sexta",
            Self::Sat => "Todo sábado",
            Self::Sun => "Todo domingo",
        }
    }
}

/// Horário `HH:MM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TimeOfDay {
    hour: u8,
    minute: u8,
}

impl TimeOfDay {
    pub fn new(hour: u8, minute: u8) -> Result<Self, String> {
        if hour > 23 || minute > 59 {
            return Err(format!("horário inválido: {hour:02}:{minute:02}"));
        }
        Ok(Self { hour, minute })
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl TryFrom<String> for TimeOfDay {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let err = || format!("horário inválido '{value}' (use HH:MM)");
        let (h, m) = value.split_once(':').ok_or_else(err)?;
        if h.len() != 2 || m.len() != 2 {
            return Err(err());
        }
        Self::new(h.parse().map_err(|_| err())?, m.parse().map_err(|_| err())?)
    }
}

impl From<TimeOfDay> for String {
    fn from(value: TimeOfDay) -> Self {
        value.to_string()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    #[default]
    Manual,
    EveryHours {
        hours: u8,
    },
    Daily {
        at: TimeOfDay,
    },
    Weekly {
        weekday: Weekday,
        at: TimeOfDay,
    },
    /// Expressão `OnCalendar` avançada (RN-82).
    Calendar {
        expr: String,
    },
}

impl Schedule {
    /// RN-80: intervalos que dividem o dia igualmente.
    pub const ALLOWED_HOURS: [u8; 7] = [1, 2, 3, 4, 6, 8, 12];
    pub const MAX_EXPR_LEN: usize = 64;

    /// Validação sem I/O. A expressão avançada ainda passa por
    /// `systemd-analyze calendar` antes de ser instalada (RN-82, SEG-22).
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::EveryHours { hours } if !Self::ALLOWED_HOURS.contains(hours) => Err(format!(
                "intervalo de {hours}h não permitido; use {:?}",
                Self::ALLOWED_HOURS
            )),
            Self::Calendar { expr } => validate_calendar_expr(expr),
            _ => Ok(()),
        }
    }

    /// Valor para `OnCalendar=`; `None` para manual.
    pub fn on_calendar(&self) -> Option<String> {
        match self {
            Self::Manual => None,
            Self::EveryHours { hours } => Some(format!("*-*-* 00/{hours}:00:00")),
            Self::Daily { at } => Some(format!("*-*-* {at}:00")),
            Self::Weekly { weekday, at } => Some(format!("{} *-*-* {at}:00", weekday.systemd())),
            Self::Calendar { expr } => Some(expr.trim().to_string()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Manual => "Manual".into(),
            Self::EveryHours { hours: 1 } => "A cada hora".into(),
            Self::EveryHours { hours } => format!("A cada {hours} horas"),
            Self::Daily { at } => format!("Diário às {at}"),
            Self::Weekly { weekday, at } => format!("{} às {at}", weekday.every_label()),
            Self::Calendar { expr } => format!("Personalizado ({expr})"),
        }
    }
}

/// SEG-22: allowlist de caracteres — impede, p.ex., quebra de linha injetando
/// diretivas no arquivo `.timer`.
fn validate_calendar_expr(expr: &str) -> Result<(), String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err("expressão de agendamento vazia".into());
    }
    if expr.len() > Schedule::MAX_EXPR_LEN {
        return Err(format!(
            "expressão com mais de {} caracteres",
            Schedule::MAX_EXPR_LEN
        ));
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || " *-:/,.~".contains(c);
    if let Some(bad) = expr.chars().find(|c| !allowed(*c)) {
        return Err(format!("caractere não permitido na expressão: {bad:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converte_para_on_calendar() {
        let at = TimeOfDay::new(2, 30).unwrap();
        assert_eq!(Schedule::Manual.on_calendar(), None);
        assert_eq!(
            Schedule::EveryHours { hours: 6 }.on_calendar().unwrap(),
            "*-*-* 00/6:00:00"
        );
        assert_eq!(
            Schedule::Daily { at }.on_calendar().unwrap(),
            "*-*-* 02:30:00"
        );
        assert_eq!(
            Schedule::Weekly {
                weekday: Weekday::Sun,
                at
            }
            .on_calendar()
            .unwrap(),
            "Sun *-*-* 02:30:00"
        );
    }

    #[test]
    fn rotulos_em_portugues() {
        let at = TimeOfDay::new(3, 15).unwrap();
        let weekly = |weekday| Schedule::Weekly { weekday, at }.label();
        assert_eq!(weekly(Weekday::Sun), "Todo domingo às 03:15");
        assert_eq!(weekly(Weekday::Mon), "Toda segunda às 03:15");
        assert_eq!(Schedule::EveryHours { hours: 1 }.label(), "A cada hora");
    }

    #[test]
    fn horario_valida_formato() {
        assert!(TimeOfDay::try_from("07:05".to_string()).is_ok());
        for bad in ["7:05", "24:00", "12:60", "1200", "aa:bb"] {
            assert!(TimeOfDay::try_from(bad.to_string()).is_err(), "{bad}");
        }
    }

    #[test]
    fn intervalo_de_horas_restrito() {
        assert!(Schedule::EveryHours { hours: 4 }.validate().is_ok());
        assert!(Schedule::EveryHours { hours: 5 }.validate().is_err());
    }

    #[test]
    fn expressao_avancada_bloqueia_injecao() {
        let ok = Schedule::Calendar {
            expr: "Mon..Fri *-*-* 09:00:00".into(),
        };
        assert!(ok.validate().is_ok());
        let injected = Schedule::Calendar {
            expr: "daily\nExecStart=/bin/sh".into(),
        };
        assert!(injected.validate().is_err());
        let quoted = Schedule::Calendar {
            expr: "daily; rm -rf ~".into(),
        };
        assert!(quoted.validate().is_err());
    }
}
