use palette::bool_mask::LazySelect;
use palette::encoding::gamma::{GammaFn, Number};
use palette::encoding::{FromLinear, IntoLinear, Srgb};
use palette::num::{Abs, Arithmetics, MulSub, One, PartialCmp, Powf, Real, Signum};
use palette::rgb::{Primaries, RgbSpace, RgbStandard};
use palette::white_point::{Any, D50, D65};
use palette::{Mat3, Yxy};

#[derive(Copy, Clone, PartialEq, Debug, Eq)]
pub struct Adobe98Gamma;
impl Number for Adobe98Gamma {
    const VALUE: f64 = 256. / 563.;
}

impl<T: Real + Powf + Signum + Abs + Clone + PartialCmp + Arithmetics + MulSub + One + Copy>
    IntoLinear<T, T> for Rec2020
where
    T::Mask: LazySelect<T>,
{
    fn into_linear(encoded: T) -> T {
        let alpha = T::from_f64(1.09929682680944);
        let beta: T = T::from_f64(0.018053968510807);

        let sign = encoded.clone().signum();
        let abs = encoded.clone().abs();

        LazySelect::lazy_select(
            abs.lt(&(beta * T::from_f64(4.5))),
            || encoded / T::from_f64(4.5),
            || sign * ((abs + alpha - T::one()) / alpha).powf(T::from_f64(1. / 0.45)),
        )
    }
}

impl<T: Real + Powf + Signum + Abs + Clone + PartialCmp + Arithmetics + One + MulSub + Copy>
    FromLinear<T, T> for Rec2020
where
    T::Mask: LazySelect<T>,
{
    fn from_linear(linear: T) -> T {
        let alpha: T = T::from_f64(1.09929682680944);
        let beta: T = T::from_f64(0.018053968510807);

        let sign = linear.clone().signum();
        let abs = linear.clone().abs();
        LazySelect::lazy_select(
            abs.lt(&beta),
            || linear * T::from_f64(4.5),
            || sign * (abs.powf(T::from_f64(0.45)) * alpha + T::one() - alpha),
        )
    }
}

impl<T: Real + Powf + Signum + Abs + Clone + PartialCmp + Arithmetics + MulSub + One>
    IntoLinear<T, T> for ProPhoto
where
    T::Mask: LazySelect<T>,
{
    fn into_linear(encoded: T) -> T {
        let e: T = T::from_f64(16. / 512.);
        let sign = encoded.clone().signum();
        let abs = encoded.clone().abs();
        LazySelect::lazy_select(
            abs.lt(&e),
            || encoded / T::from_f64(16.),
            || sign * abs.powf(T::from_f64(1.8)),
        )
    }
}

impl<T: Real + Powf + Signum + Abs + Clone + PartialCmp + Arithmetics + MulSub> FromLinear<T, T>
    for ProPhoto
where
    T::Mask: LazySelect<T>,
{
    fn from_linear(linear: T) -> T {
        let e: T = T::from_f64(1. / 512.);
        let sign = linear.clone().signum();
        let abs = linear.clone().abs();
        LazySelect::lazy_select(
            abs.lt(&e),
            || linear * T::from_f64(16.),
            || sign * abs.powf(T::from_f64(1. / 1.8)),
        )
    }
}

include!("colorspace.rs.gen");
