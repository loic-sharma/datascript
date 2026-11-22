//! The chart a `render` operator asks for (LANGUAGE.md §6), as an explicit [`RenderSpec`].
//!
//! The binder resolves every default, so the spec names every column it plots and frontends
//! never guess (D-047). Its JSON form is the contract with `web/` and VS Code (ARCHITECTURE.md,
//! "Render layer"): camelCase keys, fields that are not set are absent (never `null`), and keys
//! are only ever added.

use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};

/// A chart kind (LANGUAGE.md §6). Serialized as written in the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    #[default]
    Table,
    Timechart,
    Linechart,
    Areachart,
    Barchart,
    Columnchart,
    Piechart,
    Scatterchart,
    Card,
}

impl ChartKind {
    /// Every kind, in the order of LANGUAGE.md §6.
    pub const ALL: [ChartKind; 9] = [
        ChartKind::Table,
        ChartKind::Timechart,
        ChartKind::Linechart,
        ChartKind::Areachart,
        ChartKind::Barchart,
        ChartKind::Columnchart,
        ChartKind::Piechart,
        ChartKind::Scatterchart,
        ChartKind::Card,
    ];

    /// The name as written in `render <kind>`.
    pub fn name(self) -> &'static str {
        match self {
            ChartKind::Table => "table",
            ChartKind::Timechart => "timechart",
            ChartKind::Linechart => "linechart",
            ChartKind::Areachart => "areachart",
            ChartKind::Barchart => "barchart",
            ChartKind::Columnchart => "columnchart",
            ChartKind::Piechart => "piechart",
            ChartKind::Scatterchart => "scatterchart",
            ChartKind::Card => "card",
        }
    }

    /// The kind with this name (case-sensitive, like all keywords).
    pub fn from_name(name: &str) -> Option<ChartKind> {
        ChartKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Whether the kind has an x role (and with it `xcolumn`, `ycolumns`, `xtitle`, `ytitle`
    /// and `legend`).
    pub fn has_axes(self) -> bool {
        !matches!(self, ChartKind::Table | ChartKind::Card)
    }

    /// Whether the kind takes `series`.
    pub fn has_series(self) -> bool {
        self.has_axes() && self != ChartKind::Piechart
    }

    /// Whether the kind takes `ymin` and `ymax`.
    pub fn has_y_range(self) -> bool {
        self.has_axes() && self != ChartKind::Piechart
    }

    /// Whether the kind takes the `kind` property (stacking).
    pub fn has_stacking(self) -> bool {
        matches!(
            self,
            ChartKind::Areachart | ChartKind::Barchart | ChartKind::Columnchart
        )
    }
}

impl fmt::Display for ChartKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Whether the chart shows its legend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Legend {
    #[default]
    Visible,
    Hidden,
}

/// How an area, bar or column chart stacks its series (the `kind` property; `unstacked` is
/// [`Stacking::Default`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stacking {
    #[default]
    Default,
    Stacked,
    Stacked100,
}

/// A constant number for `ymin` or `ymax`, kept as written: `0` serializes as `0`, `0.5` as
/// `0.5`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RenderNumber {
    Long(i64),
    Real(f64),
}

impl RenderNumber {
    pub fn as_f64(self) -> f64 {
        match self {
            RenderNumber::Long(n) => n as f64,
            RenderNumber::Real(x) => x,
        }
    }
}

impl fmt::Display for RenderNumber {
    /// As result tables print numbers: integers plainly, reals in their shortest round-trip
    /// form with a decimal point (`980.0`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderNumber::Long(n) => write!(f, "{n}"),
            RenderNumber::Real(x) => write!(f, "{x:?}"),
        }
    }
}

/// The chart a final `render` asks frontends to draw, with every default resolved
/// (LANGUAGE.md §6.2). Columns are output column names of the query.
///
/// Which fields are set depends on the kind (ARCHITECTURE.md, "Render layer"): `x` for kinds
/// with an x role, `y` for every kind except `table`, `series` for kinds that take it,
/// `legend` for every kind except `table` and `card`, `stacking` for area, bar and column
/// charts; titles and `ymin`/`ymax` only when given.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct RenderSpec {
    pub kind: ChartKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ytitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend: Option<Legend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ymin: Option<RenderNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ymax: Option<RenderNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stacking: Option<Stacking>,
}

impl RenderSpec {
    /// A spec of this kind with nothing else set.
    pub fn new(kind: ChartKind) -> RenderSpec {
        RenderSpec {
            kind,
            ..RenderSpec::default()
        }
    }

    /// The one-line summary shared by golden tests and `ds explain` (TESTING.md, "Expected
    /// output rules"): `render: timechart x=month y=[revenue] series=[customerId]`.
    ///
    /// Fields print in a fixed order; `series` is left out when empty, `legend` unless it is
    /// `hidden`, `stacking` unless it is not `default`. Column names print as in the language,
    /// strings in double quotes with control characters escaped.
    pub fn summary(&self) -> String {
        let mut out = format!("render: {}", self.kind);
        if let Some(x) = &self.x {
            let _ = write!(out, " x={}", datascript_syntax::display_name(x));
        }
        if let Some(y) = &self.y {
            let _ = write!(out, " y={}", column_list(y));
        }
        if let Some(series) = self.series.as_ref().filter(|s| !s.is_empty()) {
            let _ = write!(out, " series={}", column_list(series));
        }
        for (key, value) in [
            ("title", &self.title),
            ("xtitle", &self.xtitle),
            ("ytitle", &self.ytitle),
        ] {
            if let Some(value) = value {
                let _ = write!(out, " {key}=\"{}\"", escape(value));
            }
        }
        if self.legend == Some(Legend::Hidden) {
            out.push_str(" legend=hidden");
        }
        if let Some(ymin) = self.ymin {
            let _ = write!(out, " ymin={ymin}");
        }
        if let Some(ymax) = self.ymax {
            let _ = write!(out, " ymax={ymax}");
        }
        match self.stacking {
            Some(Stacking::Stacked) => out.push_str(" stacking=stacked"),
            Some(Stacking::Stacked100) => out.push_str(" stacking=stacked100"),
            Some(Stacking::Default) | None => {}
        }
        out
    }
}

fn column_list(columns: &[String]) -> String {
    let names: Vec<String> = columns
        .iter()
        .map(|c| datascript_syntax::display_name(c))
        .collect();
    format!("[{}]", names.join(", "))
}

/// Control characters escaped as in result tables (`\n`, `\t`, `\u{1b}`).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readme_spec() -> RenderSpec {
        RenderSpec {
            x: Some("month".into()),
            y: Some(vec!["revenue".into()]),
            series: Some(vec!["customerId".into()]),
            legend: Some(Legend::Visible),
            ..RenderSpec::new(ChartKind::Timechart)
        }
    }

    #[test]
    fn json_matches_architecture_md() {
        let spec = RenderSpec {
            title: Some("Revenue".into()),
            xtitle: Some("Month".into()),
            ytitle: Some("EUR".into()),
            ymin: Some(RenderNumber::Long(0)),
            ymax: Some(RenderNumber::Long(1000)),
            stacking: Some(Stacking::Default),
            ..readme_spec()
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"timechart","x":"month","y":["revenue"],"series":["customerId"],"title":"Revenue","xtitle":"Month","ytitle":"EUR","legend":"visible","ymin":0,"ymax":1000,"stacking":"default"}"#
        );
        let back: RenderSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn unset_fields_are_absent_not_null() {
        let table = serde_json::to_string(&RenderSpec::new(ChartKind::Table)).unwrap();
        assert_eq!(table, r#"{"kind":"table"}"#);
        let card = RenderSpec {
            y: Some(vec!["a".into()]),
            ..RenderSpec::new(ChartKind::Card)
        };
        assert_eq!(
            serde_json::to_string(&card).unwrap(),
            r#"{"kind":"card","y":["a"]}"#
        );
        let stacked = RenderSpec {
            stacking: Some(Stacking::Stacked100),
            ymin: Some(RenderNumber::Real(-0.5)),
            series: Some(vec![]),
            ..RenderSpec::new(ChartKind::Columnchart)
        };
        assert_eq!(
            serde_json::to_string(&stacked).unwrap(),
            r#"{"kind":"columnchart","series":[],"ymin":-0.5,"stacking":"stacked100"}"#
        );
    }

    #[test]
    fn summary_line() {
        assert_eq!(
            readme_spec().summary(),
            "render: timechart x=month y=[revenue] series=[customerId]"
        );
        let spec = RenderSpec {
            x: Some("order id".into()),
            y: Some(vec!["a".into(), "b c".into()]),
            series: Some(vec![]),
            title: Some("T\n\"q\"".into()),
            legend: Some(Legend::Hidden),
            ymin: Some(RenderNumber::Real(0.0)),
            ymax: Some(RenderNumber::Long(5)),
            stacking: Some(Stacking::Stacked),
            ..RenderSpec::new(ChartKind::Barchart)
        };
        assert_eq!(
            spec.summary(),
            r#"render: barchart x=['order id'] y=[a, ['b c']] title="T\n"q"" legend=hidden ymin=0.0 ymax=5 stacking=stacked"#
        );
        assert_eq!(RenderSpec::new(ChartKind::Table).summary(), "render: table");
    }
}
