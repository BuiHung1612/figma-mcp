use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub struct Rgba(pub [f64; 4]);

impl Rgba {
    pub fn parse(value: &Value) -> Result<Self, String> {
        if let Some(s) = value.as_str() {
            let color =
                csscolorparser::parse(s).map_err(|e| format!("Invalid color '{s}': {e}"))?;
            return Ok(Self(color.to_array().map(|v| v as f64)));
        }
        let mut channels = [0.0, 0.0, 0.0, 1.0];
        for (i, key) in ["r", "g", "b", "a"].iter().enumerate() {
            channels[i] = if *key == "a" && value.get(key).is_none() {
                1.0
            } else {
                value
                    .get(key)
                    .and_then(Value::as_f64)
                    .ok_or_else(|| format!("Missing color channel {key}"))?
            };
            if !channels[i].is_finite() || !(0.0..=1.0).contains(&channels[i]) {
                return Err(format!("Invalid normalized color channel {key}"));
            }
        }
        Ok(Self(channels))
    }

    pub fn css(self) -> String {
        let [r, g, b, a] = self.0;
        let [r, g, b] = [r, g, b].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        if a >= 1.0 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("rgba({r}, {g}, {b}, {})", number(a))
        }
    }

    pub fn with_opacity(mut self, opacity: f64) -> Result<Self, String> {
        if !(0.0..=1.0).contains(&opacity) {
            return Err("Invalid paint opacity".into());
        }
        self.0[3] *= opacity;
        Ok(self)
    }

    pub fn matches(self, other: Self) -> bool {
        self.0
            .iter()
            .zip(other.0)
            .all(|(a, b)| (a - b).abs() <= 1.0 / 255.0 + 1e-6)
    }
}

pub fn number(value: f64) -> String {
    let rounded = (value * 1_000_000.0).round() / 1_000_000.0;
    format!("{}", if rounded == 0.0 { 0.0 } else { rounded })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn css_formats_and_alpha_are_equivalent() {
        let expected = Rgba::parse(&json!("rgba(255, 0, 0, .5)")).unwrap();
        for input in ["rgb(100% 0% 0% / 50%)", "hsl(0 100% 50% / .5)", "#ff000080"] {
            assert!(
                expected.matches(Rgba::parse(&json!(input)).unwrap()),
                "{input}"
            );
        }
        assert!(!expected.matches(Rgba::parse(&json!("#ff0000")).unwrap()));
        assert_eq!(
            Rgba::parse(&json!({"r": 1, "g": 0, "b": 0, "a": 0}))
                .unwrap()
                .css(),
            "rgba(255, 0, 0, 0)"
        );
        assert!(Rgba::parse(&json!("rgba(nonsense)")).is_err());
    }
}
