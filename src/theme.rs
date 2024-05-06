use anyhow::{anyhow, bail};
use cssparser::color::PredefinedColorSpace;
use cssparser::{BasicParseErrorKind, ParseErrorKind, Parser, ParserInput};
use cssparser_color::Color as CssColor;
use lazy_regex::{lazy_regex, Regex};
use palette::chromatic_adaptation::AdaptFrom;
use palette::white_point::{D50, D65};
use palette::{Darken, FromColor, IntoColor, Mix, Oklab, Oklaba, Srgb, WithAlpha};
use std::cmp::Ordering;
use std::io::BufRead;
use thiserror::Error;

static DIRS: once_cell::sync::Lazy<xdg::BaseDirectories> =
    once_cell::sync::Lazy::new(|| xdg::BaseDirectories::with_prefix("wattbar").unwrap());

static SECTION_RE: lazy_regex::Lazy<Regex> = lazy_regex!(r"^\s*\[\s*([a-z]+)\s*\]\s*$");

#[derive(Copy, Clone, Debug)]
pub enum ChargeState {
    Charging,
    NoCharge,
    Discharging,
}

#[derive(Copy, Clone, Debug)]
pub struct GradientStop {
    level: f32,
    fg: Oklaba,
    bg: Oklaba,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub discharging: Vec<GradientStop>,
    pub no_charge: Vec<GradientStop>,
    pub charging: Vec<GradientStop>,
}

struct Totalize<T>(T);

impl PartialEq for Totalize<f32> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Totalize<f32> {}
impl PartialOrd for Totalize<f32> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.0.total_cmp(&other.0))
    }
}
impl Ord for Totalize<f32> {
    fn cmp(&self, other: &Self) -> Ordering {
        f32::total_cmp(&self.0, &other.0)
    }
}

#[derive(Debug, Error)]
enum CssParseError {
    #[error("Internal error parsing CSS color")]
    InternalError,
    #[error("Unsupported CSS color type")]
    Unsupported,
}
impl From<()> for CssParseError {
    fn from(_: ()) -> Self {
        CssParseError::InternalError
    }
}

/// Themes
impl Theme {
    pub(crate) fn load(name: &str) -> anyhow::Result<Self> {
        let name = format!("{name}.theme");
        let path = DIRS.find_config_file(&name)
            .or_else(|| DIRS.find_data_file(&name));
        let path = if let Some(path) = path {
            path
        } else if name == "default.theme" {
            // Write out the default theme
            let path = DIRS.place_config_file(name)?;
            std::fs::write(&path, include_bytes!("../default.theme"))?;
            path
        } else {
            let mut dirs = vec![DIRS.get_config_home()];
            dirs.extend(DIRS.get_config_dirs());
            dirs.push(DIRS.get_data_home());
            dirs.extend(DIRS.get_data_dirs());
            bail!(
                "Unable to find theme {name} (it should be in one of {})",
                dirs
                    .iter()
                    .map(|buf| buf.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let file = std::fs::File::open(path)?;
        let lines = std::io::BufReader::new(file).lines();

        let mut result = Self {
            discharging: vec![],
            no_charge: vec![],
            charging: vec![],
        };

        let mut section = None;
        let mut line_no = 0;
        for line in lines {
            line_no += 1;
            let line = line?;

            if let Some(captures) = SECTION_RE.captures(&line) {
                let section_name = &captures[1];
                section = Some(match section_name.to_lowercase().as_str() {
                    "charging" => ChargeState::Charging,
                    "nocharge" => ChargeState::NoCharge,
                    "discharging" => ChargeState::Discharging,
                    _ => bail!("Invalid section name at line {line_no}: expected one of charging, nocharge, or discharging"),
                });
            } else if line.trim().is_empty() {
                continue;
            } else {
                let section =
                    result
                        .section_by_name_mut(section.ok_or_else(|| {
                            anyhow!("Unexpected gradient stop at line {line_no}")
                        })?);
                let stop = GradientStop::parse_stop(line.as_str()).map_err(|err| {
                    let mut err: cssparser::ParseError<CssParseError> = err.into();
                    err.location.line += line_no - 1;
                    anyhow!("{err}")
                })?;
                section.push(stop)
            }
        }

        result.charging.sort_by_key(|stop| Totalize(stop.level));
        result.discharging.sort_by_key(|stop| Totalize(stop.level));
        result.no_charge.sort_by_key(|stop| Totalize(stop.level));

        Ok(result)
    }

    fn section_by_name_mut(&mut self, state: ChargeState) -> &mut Vec<GradientStop> {
        match state {
            ChargeState::Charging => &mut self.charging,
            ChargeState::NoCharge => &mut self.no_charge,
            ChargeState::Discharging => &mut self.discharging,
        }
    }

    pub fn section_by_name(&self, state: ChargeState) -> &Vec<GradientStop> {
        match state {
            ChargeState::Charging => &self.charging,
            ChargeState::NoCharge => &self.no_charge,
            ChargeState::Discharging => &self.discharging,
        }
    }

    pub fn colors_at(&self, state: ChargeState, level: f32) -> (Oklaba, Oklaba) {
        // We can assume that at least one color is defined for each charge state
        let section = self.section_by_name(state);
        let mut last_state = &section[0];
        let mut next_state = &section[0];
        for state in section {
            next_state = state;
            if state.level > level {
                break;
            }
            last_state = state;
        }
        return if last_state.level == next_state.level {
            // before first iteration, after last iteration, or on a discontinuity
            (last_state.fg, last_state.bg)
        } else {
            let ratio = (level - last_state.level) / (next_state.level - last_state.level);
            let fg = last_state.fg.mix(next_state.fg, ratio);
            let bg = last_state.bg.mix(next_state.bg, ratio);
            (fg, bg)
        };
    }
}

impl GradientStop {
    fn parse_stop(line: &str) -> Result<Self, cssparser::ParseError<CssParseError>> {
        let mut input = ParserInput::new(line);
        let mut parser = Parser::new(&mut input);
        let level = parser.expect_percentage()?;
        let fg_color = CssColor::parse(&mut parser).map_err(cssparser::ParseError::into)?;
        let bg_color = CssColor::parse(&mut parser).map(Some).or_else(|err| {
            if err.kind == ParseErrorKind::Basic(BasicParseErrorKind::EndOfInput) {
                Ok(None)
            } else {
                Err(err.into())
            }
        })?;

        let fg_color = convert_color(fg_color).ok_or(cssparser::ParseError {
            kind: cssparser::ParseErrorKind::Custom(CssParseError::Unsupported),
            location: cssparser::SourceLocation { line: 1, column: 0 },
        })?;
        let bg_color = if let Some(bg_color) = bg_color {
            convert_color(bg_color).ok_or(cssparser::ParseError {
                kind: cssparser::ParseErrorKind::Custom(CssParseError::Unsupported),
                location: cssparser::SourceLocation { line: 1, column: 0 },
            })?
        } else {
            fg_color.darken(0.5)
        };

        Ok(GradientStop {
            level,
            fg: fg_color,
            bg: bg_color,
        })
    }
}

fn convert_color(color: CssColor) -> Option<palette::Oklaba> {
    use crate::colorspace::*;
    let result: Oklaba = match color {
        CssColor::CurrentColor => return None,
        CssColor::Rgba(cssparser_color::RgbaLegacy {
            red,
            green,
            blue,
            alpha,
        }) => Oklab::from_color(palette::LinSrgb::from_encoding(Srgb::new(red, green, blue)))
            .with_alpha(alpha),
        CssColor::Hsl(cssparser_color::Hsl {
            hue,
            lightness,
            saturation,
            alpha,
        }) => {
            let hsl = palette::Hsla::<palette::encoding::Srgb>::new(
                hue?,
                lightness?,
                saturation?,
                alpha.unwrap_or(1.),
            );
            palette::Oklaba::from_color(hsl)
        }
        CssColor::Hwb(cssparser_color::Hwb {
            hue,
            whiteness,
            blackness,
            alpha,
        }) => {
            let hwb = palette::Hwba::<palette::encoding::Srgb>::new(
                hue?,
                whiteness?,
                blackness?,
                alpha.unwrap_or(1.),
            );
            palette::Oklaba::from_color(hwb)
        }
        CssColor::Lab(cssparser_color::Lab {
            lightness,
            a,
            b,
            alpha,
        }) => palette::Lab::new(lightness?, a?, b?)
            .with_alpha(alpha.unwrap_or(1.))
            .into_color(),
        CssColor::Lch(cssparser_color::Lch {
            lightness,
            chroma,
            hue,
            alpha,
        }) => palette::Lcha::new(lightness?, chroma?, hue?, alpha.unwrap_or(1.)).into_color(),
        CssColor::Oklab(cssparser_color::Oklab {
            lightness,
            a,
            b,
            alpha,
        }) => palette::Oklaba::new(lightness?, a?, b?, alpha.unwrap_or(1.)),
        CssColor::Oklch(cssparser_color::Oklch {
            lightness,
            chroma,
            hue,
            alpha,
        }) => palette::Oklcha::new(lightness?, chroma?, hue?, alpha.unwrap_or(1.)).into_color(),
        CssColor::ColorFunction(cssparser_color::ColorFunction {
            color_space,
            c1,
            c2,
            c3,
            alpha,
        }) => {
            let c1 = c1?;
            let c2 = c2?;
            let c3 = c3?;
            let alpha = alpha.unwrap_or(1.);

            match color_space {
                PredefinedColorSpace::Srgb => palette::Srgba::new(c1, c2, c3, alpha).into_color(),
                PredefinedColorSpace::SrgbLinear => {
                    palette::LinSrgba::new(c1, c2, c3, alpha).into_color()
                }
                PredefinedColorSpace::DisplayP3 => {
                    palette::rgb::Rgba::<DisplayP3>::new(c1, c2, c3, alpha).into_color()
                }
                PredefinedColorSpace::A98Rgb => {
                    palette::rgb::Rgba::<Adobe98>::new(c1, c2, c3, alpha).into_color()
                }
                PredefinedColorSpace::ProphotoRgb => {
                    let pro_photo = palette::rgb::Rgba::<ProPhoto>::new(c1, c2, c3, alpha);
                    let xyz_d50 = palette::Xyza::<D50>::from_color(pro_photo);
                    let xyz_d65 = palette::Xyza::<D65>::adapt_from(xyz_d50);
                    xyz_d65.into_color()
                }

                PredefinedColorSpace::Rec2020 => {
                    palette::rgb::Rgba::<Rec2020>::new(c1, c2, c3, alpha).into_color()
                }
                PredefinedColorSpace::XyzD50 => {
                    let xyz_d50 = palette::Xyza::<D50>::new(c1, c2, c3, alpha);
                    let xyz_d65 = palette::Xyza::<D65>::adapt_from(xyz_d50);
                    xyz_d65.into_color()
                }
                PredefinedColorSpace::XyzD65 => {
                    palette::Xyza::<D65>::new(c1, c2, c3, alpha).into_color()
                }
            }
        }
    };
    Some(result)
}
