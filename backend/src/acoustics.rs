use crate::models::{ResonanceAnalysisResult, SensorReading, UrnDevice};
use chrono::Utc;
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy)]
pub enum UrnShape {
    Spherical,
    Cylindrical,
    Ellipsoidal,
    Irregular,
}

pub struct HelmholtzResonator {
    pub volume: f64,
    pub neck_radius: f64,
    pub neck_length: f64,
    pub speed_of_sound: f64,
    pub shape: UrnShape,
    pub wall_thickness: f64,
    pub rim_flange_width: f64,
}

impl HelmholtzResonator {
    pub fn new(volume: f64, neck_radius: f64, neck_length: f64, speed_of_sound: f64) -> Self {
        Self {
            volume,
            neck_radius,
            neck_length,
            speed_of_sound,
            shape: UrnShape::Spherical,
            wall_thickness: 0.01,
            rim_flange_width: 0.0,
        }
    }

    pub fn with_shape(mut self, shape: UrnShape) -> Self {
        self.shape = shape;
        self
    }

    pub fn with_rim_flange(mut self, width: f64) -> Self {
        self.rim_flange_width = width;
        self
    }

    pub fn from_device(device: &UrnDevice, speed_of_sound: f64) -> Self {
        Self::new(
            device.urn_volume,
            device.neck_radius,
            device.neck_length,
            speed_of_sound,
        )
    }

    pub fn resonance_frequency(&self) -> f64 {
        let neck_area = PI * self.neck_radius.powi(2);
        let effective_length = self.neck_length + self.end_correction();
        let denominator = 2.0 * PI * (self.volume * effective_length / neck_area).sqrt();
        self.speed_of_sound / denominator
    }

    pub fn end_correction(&self) -> f64 {
        if self.rim_flange_width > 0.0 {
            let b = self.neck_radius + self.rim_flange_width;
            let a = self.neck_radius;
            let ratio = a / b;
            let flanged_correction = 0.85 * a * (1.0 - 0.25 * ratio);
            flanged_correction
        } else {
            0.85 * self.neck_radius
        }
    }

    pub fn gain_at_frequency(&self, frequency: f64) -> f64 {
        let f0 = self.resonance_frequency();
        let q = self.quality_factor();
        let x = frequency / f0;
        let numerator = q;
        let denominator = ((1.0 - x.powi(2)).powi(2) + (x / q).powi(2)).sqrt();
        numerator / denominator
    }

    pub fn gain_db(&self, frequency: f64) -> f64 {
        let gain = self.gain_at_frequency(frequency);
        20.0 * gain.log10()
    }

    pub fn quality_factor(&self) -> f64 {
        let neck_area = PI * self.neck_radius.powi(2);
        let effective_length = self.neck_length + self.end_correction();
        let kinematic_viscosity = 1.5e-5;
        let f0 = self.resonance_frequency();
        let viscous_boundary_layer = (kinematic_viscosity / (PI * f0)).sqrt();
        let viscous_loss = 2.0 * viscous_boundary_layer / self.neck_radius
            * (effective_length / self.neck_radius).sqrt();
        let radiation_loss = (2.0 * PI * f0 * neck_area) / (self.speed_of_sound * effective_length);
        let total_loss = viscous_loss + radiation_loss;
        let q = 1.0 / total_loss.max(0.01);
        q.min(80.0).max(5.0)
    }

    pub fn finite_element_gain_correction(&self, frequency: f64, medium_density: f64) -> f64 {
        let base_gain = self.gain_db(frequency);
        let density_correction = (medium_density / 1600.0).ln() * 2.5;
        let f0 = self.resonance_frequency();
        let ratio = frequency / f0;
        let fe_correction = if ratio < 0.5 {
            -1.5 * (0.5 - ratio)
        } else if ratio > 1.5 {
            -2.0 * (ratio - 1.5)
        } else {
            0.5 * (-(ratio - 1.0).powi(2) / 0.3).exp()
        };
        base_gain + density_correction + fe_correction
    }
}

pub struct BEMCorrection {
    num_boundary_elements: usize,
    neck_discretization: Vec<(f64, f64, f64)>,
    cavity_modes: Vec<CavityMode>,
}

#[derive(Debug, Clone)]
pub struct CavityMode {
    pub frequency: f64,
    pub amplitude: f64,
    pub q_factor: f64,
    pub mode_type: CavityModeType,
}

#[derive(Debug, Clone, Copy)]
pub enum CavityModeType {
    Helmholtz,
    Axial(u32),
    Azimuthal(u32),
    Radial(u32),
}

impl BEMCorrection {
    pub fn new(urn: &HelmholtzResonator) -> Self {
        let num_elements = 36;
        let mut neck_points = Vec::with_capacity(num_elements);
        for i in 0..num_elements {
            let theta = 2.0 * PI * i as f64 / num_elements as f64;
            let r = urn.neck_radius;
            neck_points.push((r * theta.cos(), r * theta.sin(), 0.0));
        }

        let cavity_modes = Self::compute_cavity_modes(urn);

        Self {
            num_boundary_elements: num_elements,
            neck_discretization: neck_points,
            cavity_modes,
        }
    }

    fn compute_cavity_modes(urn: &HelmholtzResonator) -> Vec<CavityMode> {
        let mut modes = Vec::new();
        let f0 = urn.resonance_frequency();
        let r = (3.0 * urn.volume / (4.0 * PI)).powf(1.0 / 3.0);
        let v = urn.speed_of_sound;

        modes.push(CavityMode {
            frequency: f0,
            amplitude: 1.0,
            q_factor: urn.quality_factor(),
            mode_type: CavityModeType::Helmholtz,
        });

        for n in 1..=3 {
            let f_axial = (n as f64) * v / (2.0 * r);
            if f_axial > f0 * 0.5 && f_axial < f0 * 5.0 {
                modes.push(CavityMode {
                    frequency: f_axial,
                    amplitude: 0.3 / n as f64,
                    q_factor: urn.quality_factor() * 0.8,
                    mode_type: CavityModeType::Axial(n),
                });
            }
        }

        for m in 1..=2 {
            let f_azimuthal = (m as f64 + 0.5) * v / (PI * r);
            if f_azimuthal > f0 * 0.3 && f_azimuthal < f0 * 4.0 {
                modes.push(CavityMode {
                    frequency: f_azimuthal,
                    amplitude: 0.2 / (m as f64).sqrt(),
                    q_factor: urn.quality_factor() * 0.7,
                    mode_type: CavityModeType::Azimuthal(m),
                });
            }
        }

        for p in 1..=2 {
            let k_p = (p as f64) * PI / r;
            let f_radial = v * k_p / (2.0 * PI);
            if f_radial > f0 * 0.5 && f_radial < f0 * 6.0 {
                modes.push(CavityMode {
                    frequency: f_radial,
                    amplitude: 0.15 / p as f64,
                    q_factor: urn.quality_factor() * 0.6,
                    mode_type: CavityModeType::Radial(p),
                });
            }
        }

        modes.sort_by(|a, b| a.frequency.partial_cmp(&b.frequency).unwrap_or(std::cmp::Ordering::Equal));
        modes
    }

    pub fn bem_corrected_resonance_freq(&self, urn: &HelmholtzResonator) -> f64 {
        let f0_helmholtz = urn.resonance_frequency();
        let mut correction = 0.0;

        correction += self.neck_viscous_correction(urn);
        correction += self.flange_impedance_correction(urn);
        correction += self.higher_mode_correction(urn);
        correction += self.shape_correction(urn);

        f0_helmholtz * (1.0 + correction)
    }

    fn neck_viscous_correction(&self, urn: &HelmholtzResonator) -> f64 {
        let kinematic_viscosity = 1.5e-5;
        let f0 = urn.resonance_frequency();
        let delta = (kinematic_viscosity / (PI * f0)).sqrt();
        let d_h = 2.0 * urn.neck_radius;
        let delta_ratio = delta / d_h;
        let correction = -0.65 * delta_ratio.sqrt();
        correction
    }

    fn flange_impedance_correction(&self, urn: &HelmholtzResonator) -> f64 {
        if urn.rim_flange_width <= 0.0 {
            return 0.0;
        }
        let k = 2.0 * PI * urn.resonance_frequency() / urn.speed_of_sound;
        let a = urn.neck_radius;
        let b = a + urn.rim_flange_width;
        let ka = k * a;
        let kb = k * b;
        let radiation_impedance_ratio = 1.0 - (b / a).powi(-2) * (ka.sin() / kb.sin()).powi(2);
        let correction = -0.02 * radiation_impedance_ratio.max(0.0);
        correction
    }

    fn higher_mode_correction(&self, urn: &HelmholtzResonator) -> f64 {
        let f0 = urn.resonance_frequency();
        let mut total_correction = 0.0;
        for mode in &self.cavity_modes {
            if let CavityModeType::Helmholtz = mode.mode_type {
                continue;
            }
            let delta_f = mode.frequency - f0;
            if delta_f.abs() < f0 * 1.5 {
                let coupling = mode.amplitude * 0.05 * (-(delta_f / (f0 * 0.3)).powi(2)).exp();
                total_correction += coupling;
            }
        }
        total_correction
    }

    fn shape_correction(&self, urn: &HelmholtzResonator) -> f64 {
        match urn.shape {
            UrnShape::Spherical => 0.0,
            UrnShape::Cylindrical => -0.03,
            UrnShape::Ellipsoidal => -0.02,
            UrnShape::Irregular => 0.05,
        }
    }

    pub fn bem_corrected_gain(&self, urn: &HelmholtzResonator, frequency: f64) -> f64 {
        let mut total_gain = 0.0;

        for mode in &self.cavity_modes {
            let f = mode.frequency * (1.0 + self.shape_correction(urn));
            let x = frequency / f;
            let q = mode.q_factor;
            let mode_gain = q / ((1.0 - x.powi(2)).powi(2) + (x / q).powi(2)).sqrt();
            total_gain += mode.amplitude * mode_gain;
        }

        if total_gain < 1.0 {
            total_gain = 1.0;
        }

        20.0 * total_gain.log10()
    }

    pub fn radiation_impedance(&self, urn: &HelmholtzResonator, frequency: f64) -> (f64, f64) {
        let k = 2.0 * PI * frequency / urn.speed_of_sound;
        let a = urn.neck_radius;
        let ka = k * a;

        let r_r = if ka < 0.3 {
            (ka * ka) / 2.0
        } else {
            1.0 - 2.0 * bessel_j1(2.0 * ka) / (2.0 * ka)
        };

        let x_r = if ka < 0.3 {
            (8.0 * ka) / (3.0 * PI)
        } else {
            2.0 * struve_h1(2.0 * ka) / (2.0 * ka)
        };

        let z0 = 1.21 * urn.speed_of_sound;
        let area = PI * a * a;
        (r_r * z0 / area, x_r * z0 / area)
    }
}

fn bessel_j1(x: f64) -> f64 {
    if x.abs() < 1e-6 {
        return x / 2.0;
    }
    x.sin() / x - x.cos() / (x * x)
}

fn struve_h1(x: f64) -> f64 {
    if x < 1.0 {
        (x / PI) * (1.0 + x * x / 9.0 + x.powi(4) / 225.0)
    } else {
        -bessel_j1(x) + 2.0 / PI
    }
}

pub struct RayTracingModel {
    pub layers: Vec<MediumLayer>,
    pub max_bounces: usize,
    pub num_rays: usize,
}

#[derive(Debug, Clone)]
pub struct MediumLayer {
    pub depth: f64,
    pub thickness: f64,
    pub density: f64,
    pub sound_speed: f64,
    pub attenuation: f64,
}

impl RayTracingModel {
    pub fn new() -> Self {
        Self {
            layers: vec![
                MediumLayer { depth: 0.0, thickness: 5.0, density: 1600.0, sound_speed: 300.0, attenuation: 0.5 },
                MediumLayer { depth: 5.0, thickness: 15.0, density: 1900.0, sound_speed: 500.0, attenuation: 0.3 },
                MediumLayer { depth: 20.0, thickness: 30.0, density: 2200.0, sound_speed: 1800.0, attenuation: 0.15 },
                MediumLayer { depth: 50.0, thickness: 100.0, density: 2500.0, sound_speed: 3500.0, attenuation: 0.05 },
            ],
            max_bounces: 5,
            num_rays: 64,
        }
    }

    pub fn snells_law(angle_incidence: f64, v1: f64, v2: f64) -> Option<f64> {
        let sin_theta2 = v2 / v1 * angle_incidence.sin();
        if sin_theta2.abs() > 1.0 {
            None
        } else {
            Some(sin_theta2.asin())
        }
    }

    pub fn reflection_coefficient(
        angle_incidence: f64,
        rho1: f64,
        rho2: f64,
        v1: f64,
        v2: f64,
    ) -> f64 {
        match Self::snells_law(angle_incidence, v1, v2) {
            None => 1.0,
            Some(angle_transmission) => {
                let normal_incidence_r = (rho2 * v2 - rho1 * v1) / (rho2 * v2 + rho1 * v1);
                let angular_factor = (angle_incidence.cos() - angle_transmission.cos()).abs()
                    / (angle_incidence.cos() + angle_transmission.cos()).abs();
                normal_incidence_r.abs() * 0.5 + angular_factor * 0.5
            }
        }
    }

    pub fn transmission_coefficient(
        angle_incidence: f64,
        rho1: f64,
        rho2: f64,
        v1: f64,
        v2: f64,
    ) -> f64 {
        let r = Self::reflection_coefficient(angle_incidence, rho1, rho2, v1, v2);
        (1.0 - r * r).sqrt()
    }

    pub fn trace_ray(&self, origin: (f64, f64), angle: f64, frequency: f64) -> Vec<RayPathPoint> {
        let mut points = Vec::new();
        let mut x = origin.0;
        let mut y = origin.1;
        let mut theta = angle;
        let mut amplitude = 1.0;
        let mut layer_idx = 0;
        let mut bounces = 0;

        points.push(RayPathPoint {
            x, y,
            amplitude,
            phase: 0.0,
            layer_index: layer_idx,
        });

        for _ in 0..200 {
            if layer_idx >= self.layers.len() {
                break;
            }

            let layer = &self.layers[layer_idx];
            let dir_x = theta.sin();
            let dir_y = -theta.cos();

            let hit_bottom = if dir_y < 0.0 {
                let y_bottom = layer.depth + layer.thickness;
                let t = (y_bottom - y) / dir_y;
                Some((t, x + dir_x * t, y_bottom))
            } else {
                None
            };

            let hit_top = if dir_y > 0.0 && layer_idx > 0 {
                let y_top = layer.depth;
                let t = (y_top - y) / dir_y;
                Some((t, x + dir_x * t, y_top))
            } else {
                None
            };

            if let Some((t, hit_x, hit_y)) = hit_bottom {
                if t > 0.0 {
                    let dist = t;
                    amplitude *= (-layer.attenuation * dist).exp();
                    amplitude /= dist.max(1.0);

                    if layer_idx + 1 < self.layers.len() {
                        let next_layer = &self.layers[layer_idx + 1];
                        let incident_angle = theta;
                        let r = Self::reflection_coefficient(
                            incident_angle,
                            layer.density, next_layer.density,
                            layer.sound_speed, next_layer.sound_speed,
                        );

                        if bounces < self.max_bounces {
                            let reflect_amp = amplitude * r;
                            if reflect_amp > 0.01 {
                                bounces += 1;
                            }
                        }

                        match Self::snells_law(incident_angle, layer.sound_speed, next_layer.sound_speed) {
                            Some(trans_angle) => {
                                let t_coeff = Self::transmission_coefficient(
                                    incident_angle,
                                    layer.density, next_layer.density,
                                    layer.sound_speed, next_layer.sound_speed,
                                );
                                amplitude *= t_coeff;
                                theta = trans_angle;
                                layer_idx += 1;
                            }
                            None => {
                                theta = -theta;
                                bounces += 1;
                            }
                        }
                    }

                    x = hit_x;
                    y = hit_y;

                    let wavelength = layer.sound_speed / frequency;
                    let phase = 2.0 * PI * (dist % wavelength) / wavelength;

                    points.push(RayPathPoint {
                        x, y,
                        amplitude,
                        phase,
                        layer_index: layer_idx.min(self.layers.len() - 1),
                    });

                    continue;
                }
            }

            if let Some((t, hit_x, hit_y)) = hit_top {
                if t > 0.0 {
                    let dist = t;
                    amplitude *= (-layer.attenuation * dist).exp();

                    if layer_idx > 0 {
                        let upper_layer = &self.layers[layer_idx - 1];
                        let incident_angle = -theta;
                        let r = Self::reflection_coefficient(
                            incident_angle,
                            layer.density, upper_layer.density,
                            layer.sound_speed, upper_layer.sound_speed,
                        );

                        if bounces < self.max_bounces {
                            let reflect_amp = amplitude * r;
                            if reflect_amp > 0.01 {
                                bounces += 1;
                            }
                        }

                        match Self::snells_law(incident_angle, layer.sound_speed, upper_layer.sound_speed) {
                            Some(trans_angle) => {
                                let t_coeff = Self::transmission_coefficient(
                                    incident_angle,
                                    layer.density, upper_layer.density,
                                    layer.sound_speed, upper_layer.sound_speed,
                                );
                                amplitude *= t_coeff;
                                theta = -trans_angle;
                                layer_idx -= 1;
                            }
                            None => {
                                theta = -theta;
                                bounces += 1;
                            }
                        }
                    }

                    x = hit_x;
                    y = hit_y;

                    points.push(RayPathPoint {
                        x, y,
                        amplitude,
                        phase: 0.0,
                        layer_index: layer_idx,
                    });

                    continue;
                }
            }

            break;
        }

        points
    }

    pub fn compute_wavefront_intensity(&self, distance: f64, depth: f64, frequency: f64) -> f64 {
        let mut total_intensity = 0.0;
        let num_angles = 20;

        for i in 0..=num_angles {
            let angle = -PI / 4.0 + (PI / 2.0) * i as f64 / num_angles as f64;
            let path = self.trace_ray((0.0, 0.0), angle, frequency);
            for point in &path {
                let d = point.x.abs();
                let d_err = (d - distance).abs();
                if d_err < distance * 0.1 && (point.y - depth).abs() < 5.0 {
                    total_intensity += point.amplitude.powi(2);
                }
            }
        }

        total_intensity / num_angles as f64
    }
}

#[derive(Debug, Clone)]
pub struct RayPathPoint {
    pub x: f64,
    pub y: f64,
    pub amplitude: f64,
    pub phase: f64,
    pub layer_index: usize,
}

pub struct AcousticAnalyzer {
    speed_of_sound: f64,
    drift_warning_threshold: f64,
    drift_critical_threshold: f64,
    use_bem: bool,
}

impl AcousticAnalyzer {
    pub fn new(speed_of_sound: f64, drift_warning_percent: f64, drift_critical_percent: f64) -> Self {
        Self {
            speed_of_sound,
            drift_warning_threshold: drift_warning_percent / 100.0,
            drift_critical_threshold: drift_critical_percent / 100.0,
            use_bem: true,
        }
    }

    pub fn with_bem(mut self, enable: bool) -> Self {
        self.use_bem = enable;
        self
    }

    pub fn use_bem(&self) -> bool {
        self.use_bem
    }

    pub fn check_resonance_anomaly(&self, measured_freq: f64, theoretical_freq: f64) -> bool {
        let drift_percent = ((measured_freq - theoretical_freq) / theoretical_freq).abs() * 100.0;
        drift_percent > self.drift_warning_threshold
    }

    pub fn analyze(
        &self,
        reading: &SensorReading,
        device: &UrnDevice,
    ) -> ResonanceAnalysisResult {
        let mut resonator = HelmholtzResonator::from_device(device, self.speed_of_sound);
        resonator = resonator.with_shape(UrnShape::Ellipsoidal);
        resonator = resonator.with_rim_flange(0.01);

        let theoretical_freq = if self.use_bem {
            let bem = BEMCorrection::new(&resonator);
            bem.bem_corrected_resonance_freq(&resonator)
        } else {
            resonator.resonance_frequency()
        };

        let measured_freq = reading.resonance_frequency;
        let drift = measured_freq - theoretical_freq;
        let drift_percent = (drift / theoretical_freq).abs() * 100.0;

        let gain_db = if self.use_bem {
            let bem = BEMCorrection::new(&resonator);
            let base_gain = bem.bem_corrected_gain(&resonator, measured_freq);
            base_gain + (reading.medium_density / 1600.0).ln() * 2.5
        } else {
            resonator.finite_element_gain_correction(measured_freq, reading.medium_density)
        };

        let q_factor = resonator.quality_factor();
        let is_anomaly = drift_percent > self.drift_warning_threshold * 100.0;

        ResonanceAnalysisResult {
            timestamp: Utc::now(),
            device_id: reading.device_id,
            measured_resonance_freq: measured_freq,
            theoretical_resonance_freq: theoretical_freq,
            gain_db,
            quality_factor: q_factor,
            frequency_drift: drift,
            drift_percent,
            is_anomaly,
        }
    }

    pub fn is_critical_drift(&self, drift_percent: f64) -> bool {
        drift_percent > self.drift_critical_threshold * 100.0
    }

    pub fn is_warning_drift(&self, drift_percent: f64) -> bool {
        drift_percent > self.drift_warning_threshold * 100.0
    }
}

pub struct WavePropagation {
    pub speed: f64,
    pub attenuation_coeff: f64,
}

impl WavePropagation {
    pub fn new(speed: f64, attenuation_coeff: f64) -> Self {
        Self { speed, attenuation_coeff }
    }

    pub fn travel_time(&self, distance: f64) -> f64 {
        distance / self.speed
    }

    pub fn amplitude_attenuation(&self, initial_amplitude: f64, distance: f64) -> f64 {
        let geometric_spreading = 1.0 / distance.max(1.0);
        let material_attenuation = (-self.attenuation_coeff * distance).exp();
        initial_amplitude * geometric_spreading * material_attenuation
    }

    pub fn phase_shift(&self, distance: f64, frequency: f64) -> f64 {
        let wavelength = self.speed / frequency;
        2.0 * PI * (distance % wavelength) / wavelength
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helmholtz_resonance() {
        let resonator = HelmholtzResonator::new(0.05, 0.05, 0.1, 343.0);
        let f0 = resonator.resonance_frequency();
        assert!(f0 > 50.0 && f0 < 500.0);
    }

    #[test]
    fn test_gain_calculation() {
        let resonator = HelmholtzResonator::new(0.05, 0.05, 0.1, 343.0);
        let f0 = resonator.resonance_frequency();
        let gain = resonator.gain_at_frequency(f0);
        assert!(gain > 1.0);
    }

    #[test]
    fn test_bem_correction() {
        let resonator = HelmholtzResonator::new(0.05, 0.05, 0.1, 343.0);
        let bem = BEMCorrection::new(&resonator);
        let f_bem = bem.bem_corrected_resonance_freq(&resonator);
        let f_helm = resonator.resonance_frequency();
        assert!((f_bem - f_helm).abs() / f_helm < 0.15);
    }

    #[test]
    fn test_snells_law() {
        let theta = RayTracingModel::snells_law(0.0, 300.0, 1500.0);
        assert!(theta.is_some());
        assert!(theta.unwrap().abs() < 0.01);
    }

    #[test]
    fn test_reflection_coefficient() {
        let r = RayTracingModel::reflection_coefficient(0.0, 1000.0, 2000.0, 300.0, 1500.0);
        assert!((0.0..=1.0).contains(&r));
    }

    #[test]
    fn test_ray_tracing() {
        let model = RayTracingModel::new();
        let path = model.trace_ray((0.0, 1.0), 0.0, 200.0);
        assert!(!path.is_empty());
    }
}
