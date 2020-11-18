//! Sky: Ported to Rust from https://github.com/Tw1ddle/Sky-Shader/blob/master/src/shaders/glsl/sky.fragment
//! Clouds: Ported to Rust from IQ's https://www.shadertoy.com/view/XslGRr

#![cfg_attr(target_arch = "spirv", no_std)]
#![feature(lang_items)]
#![feature(register_attr)]
#![register_attr(spirv)]

use core::f32::consts::PI;
use shared::*;
use spirv_std::glam::{const_vec3, Vec2, Vec3, Vec4};
use spirv_std::{Input, MathExt, Output, PushConstant};

const DEPOLARIZATION_FACTOR: f32 = 0.035;
const MIE_COEFFICIENT: f32 = 0.005;
const MIE_DIRECTIONAL_G: f32 = 0.8;
const MIE_K_COEFFICIENT: Vec3 = const_vec3!([0.686, 0.678, 0.666]);
const MIE_V: f32 = 4.0;
const MIE_ZENITH_LENGTH: f32 = 1.25e3;
const NUM_MOLECULES: f32 = 2.542e25f32;
const PRIMARIES: Vec3 = const_vec3!([6.8e-7f32, 5.5e-7f32, 4.5e-7f32]);
const RAYLEIGH: f32 = 1.0;
const RAYLEIGH_ZENITH_LENGTH: f32 = 8.4e3;
const REFRACTIVE_INDEX: f32 = 1.0003;
const SUN_ANGULAR_DIAMETER_DEGREES: f32 = 0.0093333;
const SUN_INTENSITY_FACTOR: f32 = 1000.0;
const SUN_INTENSITY_FALLOFF_STEEPNESS: f32 = 1.5;
const TURBIDITY: f32 = 2.0;

fn total_rayleigh(lambda: Vec3) -> Vec3 {
    (8.0 * PI.pow(3.0)
        * (REFRACTIVE_INDEX.pow(2.0) - 1.0).pow(2.0)
        * (6.0 + 3.0 * DEPOLARIZATION_FACTOR))
        / (3.0 * NUM_MOLECULES * pow(lambda, 4.0) * (6.0 - 7.0 * DEPOLARIZATION_FACTOR))
}

fn total_mie(lambda: Vec3, k: Vec3, t: f32) -> Vec3 {
    let c = 0.2 * t * 10e-18;
    0.434 * c * PI * pow((2.0 * PI) / lambda, MIE_V - 2.0) * k
}

fn rayleigh_phase(cos_theta: f32) -> f32 {
    (3.0 / (16.0 * PI)) * (1.0 + cos_theta.pow(2.0))
}

fn henyey_greenstein_phase(cos_theta: f32, g: f32) -> f32 {
    (1.0 / (4.0 * PI)) * ((1.0 - g.pow(2.0)) / (1.0 - 2.0 * g * cos_theta + g.pow(2.0)).pow(1.5))
}

fn sun_intensity(zenith_angle_cos: f32) -> f32 {
    let cutoff_angle = PI / 1.95; // Earth shadow hack
    SUN_INTENSITY_FACTOR
        * 0.0f32.max(
            1.0 - (-((cutoff_angle - acos_approx(zenith_angle_cos))
                / SUN_INTENSITY_FALLOFF_STEEPNESS))
                .exp(),
        )
}

fn sky(dir: Vec3, sun_position: Vec3) -> Vec3 {
    let up = Vec3::new(0.0, 1.0, 0.0);
    let sunfade = 1.0 - (1.0 - (sun_position.y() / 450000.0).exp()).saturate();
    let rayleigh_coefficient = RAYLEIGH - (1.0 * (1.0 - sunfade));
    let beta_r = total_rayleigh(PRIMARIES) * rayleigh_coefficient;

    // Mie coefficient
    let beta_m = total_mie(PRIMARIES, MIE_K_COEFFICIENT, TURBIDITY) * MIE_COEFFICIENT;

    // Optical length, cutoff angle at 90 to avoid singularity
    let zenith_angle = acos_approx(up.dot(dir).max(0.0));
    let denom = (zenith_angle).cos() + 0.15 * (93.885 - ((zenith_angle * 180.0) / PI)).pow(-1.253);

    let s_r = RAYLEIGH_ZENITH_LENGTH / denom;
    let s_m = MIE_ZENITH_LENGTH / denom;

    // Combined extinction factor
    let fex = exp(-(beta_r * s_r + beta_m * s_m));

    // In-scattering
    let sun_direction = sun_position.normalize();
    let cos_theta = dir.dot(sun_direction);
    let beta_r_theta = beta_r * rayleigh_phase(cos_theta * 0.5 + 0.5);

    let beta_m_theta = beta_m * henyey_greenstein_phase(cos_theta, MIE_DIRECTIONAL_G);
    let sun_e = sun_intensity(sun_direction.dot(up));
    let mut lin = pow(
        sun_e * ((beta_r_theta + beta_m_theta) / (beta_r + beta_m)) * (Vec3::splat(1.0) - fex),
        1.5,
    );

    lin *= Vec3::splat(1.0).lerp(
        pow(
            sun_e * ((beta_r_theta + beta_m_theta) / (beta_r + beta_m)) * fex,
            0.5,
        ),
        ((1.0 - up.dot(sun_direction)).pow(5.0)).saturate(),
    );

    // Composition + solar disc
    let sun_angular_diameter_cos = SUN_ANGULAR_DIAMETER_DEGREES.cos();
    let sundisk = my_smoothstep(
        sun_angular_diameter_cos,
        sun_angular_diameter_cos + 0.00002,
        cos_theta,
    );
    let mut l0 = 0.1 * fex;
    l0 += sun_e * 19000.0 * fex * sundisk;

    lin + l0
}

fn map(pos: Vec3, num: u32, time: f32) -> f32 {
    let q = pos - Vec3::new(0.0, 0.1, 1.0) * time;
    let mut f = 1.0;
    let mut weight = 1.0;

    let mut i = 0;
    while i < num {
        f += 0.5 / weight * noise(weight * q);

        i += 1;
        weight *= 2.1;
    }

    (1.5 - pos.y() - 2. + 1.75 * f).min(1.0).max(0.0)
}

#[allow(clippy::too_many_arguments)]
fn raymarch_cloud_layer(
    steps: u32,
    n: u32,
    ro: Vec3,
    rd: Vec3,
    sun_dir: Vec3,
    bg_col: Vec3,
    sum: &mut Vec4,
    t: &mut f32,
    time: f32,
) {
    let mut i = 0;
    while i < steps {
        let pos = ro + (*t) * rd;
        if sum.w() > 0.99 {
            break;
        }
        let den = map(pos, n, time);
        if den > 0.01 {
            let dif = ((den - map(pos + 0.3 * sun_dir, n, time)) / 0.6)
                .max(0.0)
                .min(1.0);

            let lin = Vec3::new(0.65, 0.7, 0.75) * 1.4 + Vec3::new(1.0, 0.6, 0.3) * dif;
            let mut col = Vec3::new(1.0, 0.95, 0.8)
                .lerp(Vec3::new(0.25, 0.3, 0.35), den)
                .extend(den);
            col *= lin.extend(1.0);
            col = col.lerp(bg_col.extend(col.w()), 1.0 - (-0.003 * (*t) * (*t)).exp());
            col.set_w(col.w() * 0.4);

            col.set_x(col.x() * col.w());
            col.set_y(col.y() * col.w());
            col.set_z(col.z() * col.w());

            *sum += col * (1.0 - sum.w());
        }
        *t += 0.05f32.max(0.02 * (*t));

        i += 1;
    }
}

fn raymarch_clouds(ro: Vec3, rd: Vec3, sun_dir: Vec3, bg_col: Vec3, time: f32) -> Vec4 {
    let mut sum = Vec4::zero();
    let mut t = 0.0;

    raymarch_cloud_layer(40, 5, ro, rd, sun_dir, bg_col, &mut sum, &mut t, time);
    raymarch_cloud_layer(40, 4, ro, rd, sun_dir, bg_col, &mut sum, &mut t, time);
    raymarch_cloud_layer(30, 3, ro, rd, sun_dir, bg_col, &mut sum, &mut t, time);
    raymarch_cloud_layer(30, 2, ro, rd, sun_dir, bg_col, &mut sum, &mut t, time);

    let sky_power = 2.5f32;
    Vec4::new(
        sum.x() * sky_power,
        sum.y() * sky_power,
        sum.z() * sky_power,
        sum.w(),
    )
}

pub fn fs(constants: &ShaderConstants, frag_coord: Vec2) -> Vec4 {
    let mut uv = (frag_coord - 0.5 * Vec2::new(constants.width as f32, constants.height as f32))
        / constants.height as f32;
    uv.set_y(-uv.y());

    let eye_pos = Vec3::new(0.0, 2.0, 0.0);
    let sun_pos = Vec3::new(0.0, 75.0, -1000.0);
    let dir = get_ray_dir(uv, eye_pos, sun_pos);
    let sun_dir = sun_pos.normalize();

    // evaluate Preetham sky model
    let color = sky(dir, sun_pos);

    // raymarch clouds
    let clouds = raymarch_clouds(eye_pos, dir, sun_dir, color, constants.time);
    let w = clouds.w();
    let color = color * (1.0 - w) + Vec3::new(clouds.x(), clouds.y(), clouds.z());

    // Tonemapping
    let color = color.max(Vec3::splat(0.0)).min(Vec3::splat(1024.0));

    tonemap(color).extend(1.0)
}

#[allow(unused_attributes)]
#[spirv(fragment)]
pub fn main_fs(
    #[spirv(frag_coord)] in_frag_coord: Input<Vec4>,
    #[spirv(push_constant)] constants: PushConstant<ShaderConstants>,
    mut output: Output<Vec4>,
) {
    let constants = constants.load();

    let frag_coord = Vec2::new(in_frag_coord.load().x(), in_frag_coord.load().y());
    let color = fs(&constants, frag_coord);
    output.store(color);
}

#[allow(unused_attributes)]
#[spirv(vertex)]
pub fn main_vs(
    #[spirv(vertex_index)] vert_idx: Input<i32>,
    #[spirv(position)] mut builtin_pos: Output<Vec4>,
) {
    let vert_idx = vert_idx.load();

    // Create a "full screen triangle" by mapping the vertex index.
    // ported from https://www.saschawillems.de/blog/2016/08/13/vulkan-tutorial-on-rendering-a-fullscreen-quad-without-buffers/
    let uv = Vec2::new(((vert_idx << 1) & 2) as f32, (vert_idx & 2) as f32);
    let pos = 2.0 * uv - Vec2::one();

    builtin_pos.store(pos.extend(0.0).extend(1.0));
}

#[cfg(all(not(test), target_arch = "spirv"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[cfg(all(not(test), target_arch = "spirv"))]
#[lang = "eh_personality"]
extern "C" fn rust_eh_personality() {}