#!/usr/bin/env python3

# This tool is used to generate src/colorspace.rs.gen
# Simply run it and redirect it to the appropriate file.

import numpy as np

def whitepoint(x,y):
    return np.matrix([[x/y],[1],[(1-x-y)/y]])

WhitePoints = {
  "D65": whitepoint(0.31272, 0.32903),
  "D50": whitepoint(0.3457, 0.3585),
}

def calc_RGB_to_XYZ(wp, xr, yr, xg,yg, xb,yb):
    Xr = xr/yr
    Yr = 1
    Zr = (1-xr-yr)/yr
    Xg = xg/yg
    Yg = 1
    Zg = (1-xg-yg)/yg
    Xb = xb/yb
    Yb = 1
    Zb = (1-xb-yb)/yb
    xyz = np.matrix([[Xr,Xg,Xb],[Yr,Yg,Yb],[Zr,Zg,Zb]])
    S = np.linalg.inv(xyz) * wp
    M = np.multiply(xyz, S.transpose())
    return M


def mkRust(name, wp, xr, yr, xg,yg, xb,yb, *, transfer="Srgb"):
    M = calc_RGB_to_XYZ(WhitePoints[wp], xr, yr, xg,yg, xb,yb)
    Yr = M[1,0]
    Yg = M[1,1]
    Yb = M[1,2]

    r2x = ", ".join(repr(x) for x in np.ravel(M))
    x2r = ", ".join(repr(x) for x in np.ravel(np.linalg.inv(M)))

    print(f"""
pub struct {name};
impl<T: Real> Primaries<T> for {name} {{
   fn red()   -> Yxy<Any, T> {{ Yxy::new(T::from_f64({xr}), T::from_f64({yr}), T::from_f64({Yr})) }}
   fn green() -> Yxy<Any, T> {{ Yxy::new(T::from_f64({xg}), T::from_f64({yr}), T::from_f64({Yr})) }}
   fn blue()  -> Yxy<Any, T> {{ Yxy::new(T::from_f64({xb}), T::from_f64({yr}), T::from_f64({Yr})) }}
}}

impl RgbSpace for {name} {{
   type Primaries = {name};
   type WhitePoint = {wp};

   fn rgb_to_xyz_matrix() -> Option<Mat3<f64>> {{
       Some([{r2x}])
    }}
   fn xyz_to_rgb_matrix() -> Option<Mat3<f64>> {{
       Some([{x2r}])
    }}
}}

impl RgbStandard for {name} {{
   type Space = {name};
   type TransferFn = {transfer};
}}
""")

mkRust("DisplayP3", "D65",  0.680,0.320,  0.265,0.690,  0.150,0.060)
mkRust("Adobe98", "D65",    0.640,0.330,  0.210,0.710,  0.150,0.060, transfer="GammaFn<Adobe98Gamma>")
mkRust("ProPhoto", "D50",
       0.734699,0.265301,
       0.159597,0.840403,
       0.036598,0.000105,
       transfer="ProPhoto")
mkRust("Rec2020", "D65",    0.708,0.292,  0.170,0.797,  0.131,0.046, transfer="Rec2020")

