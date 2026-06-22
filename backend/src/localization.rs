use crate::models::{SensorReading, SourceLocalizationResult, UrnDevice};
use chrono::Utc;
use std::collections::HashMap;
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BeamformingMethod {
    DelayAndSum,
    MVDR,
    MUSIC,
}

pub struct Beamformer {
    pub sound_speed: f64,
    pub resolution: f64,
    pub max_distance: f64,
    pub confidence_threshold: f64,
    pub method: BeamformingMethod,
    pub diagonal_loading: f64,
    pub multipath_suppression: bool,
}

impl Beamformer {
    pub fn new(sound_speed: f64, resolution: f64, max_distance: f64, confidence_threshold: f64) -> Self {
        Self {
            sound_speed,
            resolution,
            max_distance,
            confidence_threshold,
            method: BeamformingMethod::MVDR,
            diagonal_loading: 1e-3,
            multipath_suppression: true,
        }
    }

    pub fn with_method(mut self, method: BeamformingMethod) -> Self {
        self.method = method;
        self
    }

    pub fn with_diagonal_loading(mut self, level: f64) -> Self {
        self.diagonal_loading = level;
        self
    }

    pub fn with_multipath_suppression(mut self, enable: bool) -> Self {
        self.multipath_suppression = enable;
        self
    }

    pub fn locate_source(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        source_id: u64,
    ) -> Option<SourceLocalizationResult> {
        if readings.len() < 3 {
            return None;
        }

        let tdoa_matrix = self.compute_tdoa_matrix(readings);
        let (best_x, best_y, best_z, max_power) = match self.method {
            BeamformingMethod::DelayAndSum => self.delay_and_sum(readings, &tdoa_matrix),
            BeamformingMethod::MVDR => self.mvdr_beamform(readings, &tdoa_matrix),
            BeamformingMethod::MUSIC => self.music_spectrum(readings, &tdoa_matrix),
        };

        let center_x = readings.iter().map(|(_, d)| d.deployment_x).sum::<f64>() / readings.len() as f64;
        let center_y = readings.iter().map(|(_, d)| d.deployment_y).sum::<f64>() / readings.len() as f64;

        let dx = best_x - center_x;
        let dy = best_y - center_y;
        let dz = best_z;

        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        let bearing = dy.atan2(dx).to_degrees();
        let bearing_normalized = if bearing < 0.0 { bearing + 360.0 } else { bearing };
        let elevation = dz.atan2((dx * dx + dy * dy).sqrt()).to_degrees();

        let normalized_power = (max_power / readings.len() as f64).min(1.0).max(0.0);
        let confidence = self.calculate_confidence(readings.len(), normalized_power, distance);

        let used_devices = readings.iter().map(|(r, _)| r.device_id).collect();

        Some(SourceLocalizationResult {
            timestamp: Utc::now(),
            source_id,
            source_x: best_x,
            source_y: best_y,
            source_z: best_z,
            bearing_angle: bearing_normalized,
            elevation_angle: elevation,
            distance_estimate: distance,
            confidence,
            tdoa_matrix: tdoa_matrix.clone(),
            beamformed_power: max_power,
            used_devices,
        })
    }

    fn compute_tdoa_matrix(&self, readings: &[(SensorReading, UrnDevice)]) -> Vec<Vec<f64>> {
        let n = readings.len();
        let mut matrix = vec![vec![0.0; n]; n];

        for i in 0..n {
            for j in 0..n {
                if i != j {
                    let dist_i = self.estimate_distance_from_spl(&readings[i].0);
                    let dist_j = self.estimate_distance_from_spl(&readings[j].0);
                    let tdoa = (dist_i - dist_j) / self.sound_speed;
                    matrix[i][j] = tdoa;
                }
            }
        }
        matrix
    }

    fn estimate_distance_from_spl(&self, reading: &SensorReading) -> f64 {
        let reference_spl = 94.0;
        let reference_distance = 1.0;
        let attenuation = 6.0;
        let spl_diff = reference_spl - reading.sound_pressure_level;
        reference_distance * 10.0_f64.powf(spl_diff / attenuation)
    }

    fn delay_and_sum(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        _tdoa_matrix: &[Vec<f64>],
    ) -> (f64, f64, f64, f64) {
        let num_points = (self.max_distance / self.resolution) as usize;
        let mut max_power = 0.0;
        let mut best_x = 0.0;
        let mut best_y = 0.0;
        let mut best_z = 0.0;

        let center_x = readings.iter().map(|(_, d)| d.deployment_x).sum::<f64>() / readings.len() as f64;
        let center_y = readings.iter().map(|(_, d)| d.deployment_y).sum::<f64>() / readings.len() as f64;

        for i in 0..num_points {
            let angle = 2.0 * PI * i as f64 / num_points as f64;
            for r in (0..num_points).step_by(5) {
                let radius = r as f64 * self.resolution * 5.0;
                if radius < 1.0 || radius > self.max_distance {
                    continue;
                }

                let test_x = center_x + radius * angle.cos();
                let test_y = center_y + radius * angle.sin();
                let test_z = -radius * 0.3;

                let power = self.compute_beam_power(readings, test_x, test_y, test_z);
                if power > max_power {
                    max_power = power;
                    best_x = test_x;
                    best_y = test_y;
                    best_z = test_z;
                }
            }
        }

        (best_x, best_y, best_z, max_power)
    }

    fn compute_beam_power(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        x: f64,
        y: f64,
        z: f64,
    ) -> f64 {
        let mut total_power = 0.0;
        let reference_device = &readings[0].1;
        let reference_dist = Self::distance_3d(
            reference_device.deployment_x,
            reference_device.deployment_y,
            reference_device.deployment_z,
            x, y, z,
        );

        for (reading, device) in readings {
            let dist = Self::distance_3d(
                device.deployment_x,
                device.deployment_y,
                device.deployment_z,
                x, y, z,
            );
            let time_diff = (dist - reference_dist) / self.sound_speed;
            let phase = 2.0 * PI * reading.resonance_frequency * time_diff;
            let amplitude = reading.sound_pressure_level * 10.0_f64.powf(-dist / 100.0);
            total_power += amplitude * phase.cos();
        }

        total_power.abs()
    }

    fn mvdr_beamform(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        _tdoa_matrix: &[Vec<f64>],
    ) -> (f64, f64, f64, f64) {
        let n = readings.len();
        let cov_matrix = self.estimate_covariance_matrix(readings);
        let inv_cov = self.matrix_inverse_diag(&cov_matrix, n);

        let num_angles = (2.0 * PI / self.resolution * 20.0) as usize;
        let mut max_power = 0.0;
        let mut best_x = 0.0;
        let mut best_y = 0.0;
        let mut best_z = 0.0;

        let center_x = readings.iter().map(|(_, d)| d.deployment_x).sum::<f64>() / n as f64;
        let center_y = readings.iter().map(|(_, d)| d.deployment_y).sum::<f64>() / n as f64;

        let radial_steps = 50;
        for i in 0..num_angles {
            let angle = 2.0 * PI * i as f64 / num_angles as f64;

            for r_step in 0..radial_steps {
                let radius = self.max_distance * (r_step as f64 + 1.0) / radial_steps as f64;
                if radius < 5.0 {
                    continue;
                }

                let test_x = center_x + radius * angle.cos();
                let test_y = center_y + radius * angle.sin();
                let test_z = -radius * 0.3;

                let steering = self.steering_vector(readings, test_x, test_y, test_z);
                let power = self.mvdr_power(&steering, &inv_cov, n);

                if self.multipath_suppression {
                    let mirrored_power = self.multipath_reflected_power(
                        readings, test_x, test_y, test_z, &inv_cov, n,
                    );
                    let adjusted_power = power - 0.3 * mirrored_power;
                    if adjusted_power > max_power {
                        max_power = adjusted_power.max(0.0);
                        best_x = test_x;
                        best_y = test_y;
                        best_z = test_z;
                    }
                } else if power > max_power {
                    max_power = power;
                    best_x = test_x;
                    best_y = test_y;
                    best_z = test_z;
                }
            }
        }

        (best_x, best_y, best_z, max_power)
    }

    fn estimate_covariance_matrix(&self, readings: &[(SensorReading, UrnDevice)]) -> Vec<Vec<f64>> {
        let n = readings.len();
        let mut cov = vec![vec![0.0; n]; n];

        let freq = readings.iter().map(|(r, _)| r.resonance_frequency).sum::<f64>() / n as f64;

        for i in 0..n {
            for j in 0..n {
                if i == j {
                    let spl = readings[i].0.sound_pressure_level;
                    cov[i][j] = 10.0_f64.powf(spl / 20.0).powi(2);
                } else {
                    let di = self.estimate_distance_from_spl(&readings[i].0);
                    let dj = self.estimate_distance_from_spl(&readings[j].0);
                    let path_diff = (di - dj).abs();
                    let phase = 2.0 * PI * freq * path_diff / self.sound_speed;
                    let amp_i = 10.0_f64.powf(readings[i].0.sound_pressure_level / 20.0);
                    let amp_j = 10.0_f64.powf(readings[j].0.sound_pressure_level / 20.0);
                    cov[i][j] = amp_i * amp_j * phase.cos() * 0.6;
                }
            }
        }

        for i in 0..n {
            cov[i][i] += self.diagonal_loading * cov[i][i];
        }

        cov
    }

    fn matrix_inverse_diag(&self, matrix: &[Vec<f64>], n: usize) -> Vec<Vec<f64>> {
        let mut aug = vec![vec![0.0; 2 * n]; n];
        for i in 0..n {
            for j in 0..n {
                aug[i][j] = matrix[i][j];
            }
            aug[i][n + i] = 1.0;
        }

        for col in 0..n {
            let mut max_row = col;
            let mut max_val = aug[col][col].abs();
            for row in (col + 1)..n {
                if aug[row][col].abs() > max_val {
                    max_val = aug[row][col].abs();
                    max_row = row;
                }
            }
            if max_row != col {
                aug.swap(col, max_row);
            }

            let pivot = aug[col][col];
            if pivot.abs() < 1e-10 {
                return matrix.to_vec();
            }
            for j in 0..(2 * n) {
                aug[col][j] /= pivot;
            }

            for row in 0..n {
                if row != col {
                    let factor = aug[row][col];
                    for j in 0..(2 * n) {
                        aug[row][j] -= factor * aug[col][j];
                    }
                }
            }
        }

        let mut inv = vec![vec![0.0; n]; n];
        for i in 0..n {
            for j in 0..n {
                inv[i][j] = aug[i][n + j];
            }
        }
        inv
    }

    fn steering_vector(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        x: f64,
        y: f64,
        z: f64,
    ) -> Vec<f64> {
        let n = readings.len();
        let mut v = vec![0.0; n];
        let ref_device = &readings[0].1;
        let ref_dist = Self::distance_3d(
            ref_device.deployment_x,
            ref_device.deployment_y,
            ref_device.deployment_z,
            x, y, z,
        );
        let freq = readings.iter().map(|(r, _)| r.resonance_frequency).sum::<f64>() / n as f64;

        for (i, (_, device)) in readings.iter().enumerate() {
            let dist = Self::distance_3d(
                device.deployment_x,
                device.deployment_y,
                device.deployment_z,
                x, y, z,
            );
            let phase = 2.0 * PI * freq * (dist - ref_dist) / self.sound_speed;
            v[i] = phase.cos();
        }
        v
    }

    fn mvdr_power(&self, steering: &[f64], inv_cov: &[Vec<f64>], n: usize) -> f64 {
        let mut num = 0.0;
        for i in 0..n {
            num += steering[i] * steering[i];
        }

        let mut denom = 0.0;
        for i in 0..n {
            for j in 0..n {
                denom += steering[i] * inv_cov[i][j] * steering[j];
            }
        }

        if denom.abs() < 1e-10 {
            0.0
        } else {
            num / denom
        }
    }

    fn multipath_reflected_power(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        x: f64,
        y: f64,
        z: f64,
        inv_cov: &[Vec<f64>],
        n: usize,
    ) -> f64 {
        let reflected_z = -z;
        let reflected_steering = self.steering_vector(readings, x, y, reflected_z);
        self.mvdr_power(&reflected_steering, inv_cov, n)
    }

    fn music_spectrum(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        _tdoa_matrix: &[Vec<f64>],
    ) -> (f64, f64, f64, f64) {
        let n = readings.len();
        let cov = self.estimate_covariance_matrix(readings);
        let (eigenvalues, eigenvectors) = self.eigen_decomposition(&cov, n);

        let num_sources = 1.min(n - 1);
        let mut sorted_indices: Vec<usize> = (0..n).collect();
        sorted_indices.sort_by(|&a, &b| eigenvalues[b].partial_cmp(&eigenvalues[a]).unwrap_or(std::cmp::Ordering::Equal));

        let noise_subspace: Vec<Vec<f64>> = sorted_indices
            .iter()
            .skip(num_sources)
            .map(|&idx| eigenvectors[idx].clone())
            .collect();

        let num_angles = 120;
        let mut max_spectrum = 0.0;
        let mut best_x = 0.0;
        let mut best_y = 0.0;
        let mut best_z = 0.0;

        let center_x = readings.iter().map(|(_, d)| d.deployment_x).sum::<f64>() / n as f64;
        let center_y = readings.iter().map(|(_, d)| d.deployment_y).sum::<f64>() / n as f64;

        for i in 0..num_angles {
            let angle = 2.0 * PI * i as f64 / num_angles as f64;
            for r_step in 0..40 {
                let radius = self.max_distance * (r_step as f64 + 1.0) / 40.0;
                if radius < 5.0 { continue; }

                let test_x = center_x + radius * angle.cos();
                let test_y = center_y + radius * angle.sin();
                let test_z = -radius * 0.3;

                let steering = self.steering_vector(readings, test_x, test_y, test_z);
                let spectrum = self.music_spectrum_value(&steering, &noise_subspace);

                if spectrum > max_spectrum {
                    max_spectrum = spectrum;
                    best_x = test_x;
                    best_y = test_y;
                    best_z = test_z;
                }
            }
        }

        (best_x, best_y, best_z, max_spectrum.sqrt())
    }

    fn eigen_decomposition(&self, matrix: &[Vec<f64>], n: usize) -> (Vec<f64>, Vec<Vec<f64>>) {
        let mut a = matrix.to_vec();
        let mut eigenvalues = vec![0.0; n];
        let mut eigenvectors = vec![vec![0.0; n]; n];
        for i in 0..n { eigenvectors[i][i] = 1.0; }

        for _ in 0..50 {
            let mut max_off = 0.0;
            let mut p = 0;
            let mut q = 1;
            for i in 0..n {
                for j in (i + 1)..n {
                    if a[i][j].abs() > max_off {
                        max_off = a[i][j].abs();
                        p = i;
                        q = j;
                    }
                }
            }

            if max_off < 1e-8 { break; }

            let theta = 0.5 * (a[q][q] - a[p][p]).atan2(2.0 * a[p][q]);
            let c = theta.cos();
            let s = theta.sin();

            let app = a[p][p];
            let aqq = a[q][q];
            let apq = a[p][q];

            a[p][p] = c * c * app - 2.0 * s * c * apq + s * s * aqq;
            a[q][q] = s * s * app + 2.0 * s * c * apq + c * c * aqq;
            a[p][q] = 0.0;
            a[q][p] = 0.0;

            for i in 0..n {
                if i != p && i != q {
                    let aip = a[i][p];
                    let aiq = a[i][q];
                    a[i][p] = c * aip - s * aiq;
                    a[p][i] = a[i][p];
                    a[i][q] = s * aip + c * aiq;
                    a[q][i] = a[i][q];
                }
            }

            for i in 0..n {
                let vip = eigenvectors[i][p];
                let viq = eigenvectors[i][q];
                eigenvectors[i][p] = c * vip - s * viq;
                eigenvectors[i][q] = s * vip + c * viq;
            }
        }

        for i in 0..n {
            eigenvalues[i] = a[i][i];
        }

        (eigenvalues, eigenvectors)
    }

    fn music_spectrum_value(&self, steering: &[f64], noise_subspace: &[Vec<f64>]) -> f64 {
        let mut proj = 0.0;
        for noise_vec in noise_subspace {
            let mut dot = 0.0;
            for i in 0..steering.len() {
                dot += steering[i] * noise_vec[i];
            }
            proj += dot * dot;
        }

        if proj < 1e-10 {
            1e10
        } else {
            1.0 / proj
        }
    }

    fn distance_3d(x1: f64, y1: f64, z1: f64, x2: f64, y2: f64, z2: f64) -> f64 {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let dz = z2 - z1;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    fn calculate_confidence(&self, num_sensors: usize, power_ratio: f64, distance: f64) -> f64 {
        let sensor_factor = (num_sensors as f64 / 6.0).min(1.0);
        let distance_factor = (1.0 - distance / self.max_distance).max(0.2);
        let power_factor = power_ratio.min(1.0);

        let method_bonus = match self.method {
            BeamformingMethod::DelayAndSum => 0.0,
            BeamformingMethod::MVDR => 0.1,
            BeamformingMethod::MUSIC => 0.05,
        };

        let conf = sensor_factor * distance_factor * power_factor + method_bonus;
        conf.min(0.98).max(0.05)
    }
}

pub struct TDOALocalizer {
    sound_speed: f64,
}

impl TDOALocalizer {
    pub fn new(sound_speed: f64) -> Self {
        Self { sound_speed }
    }

    pub fn multilaterate_2d(
        &self,
        devices: &[UrnDevice],
        tdoa_values: &HashMap<(usize, usize), f64>,
    ) -> Option<(f64, f64, f64)> {
        if devices.len() < 3 {
            return None;
        }

        let reference_idx = 0;
        let ref_device = &devices[reference_idx];

        let mut best_pos = (0.0, 0.0, 0.0);
        let mut min_error = f64::MAX;

        let search_range = 1000.0;
        let grid_steps = 100;
        let step = search_range * 2.0 / grid_steps as f64;

        for i in 0..grid_steps {
            let x = ref_device.deployment_x - search_range + i as f64 * step;
            for j in 0..grid_steps {
                let y = ref_device.deployment_y - search_range + j as f64 * step;

                let error = self.calculate_tdoa_error(devices, tdoa_values, reference_idx, x, y, 0.0);
                if error < min_error {
                    min_error = error;
                    best_pos = (x, y, 0.0);
                }
            }
        }

        let confidence = (1.0 - min_error / (devices.len() as f64 * 0.01)).max(0.1).min(0.99);
        Some((best_pos.0, best_pos.1, confidence))
    }

    fn calculate_tdoa_error(
        &self,
        devices: &[UrnDevice],
        tdoa_values: &HashMap<(usize, usize), f64>,
        _ref_idx: usize,
        x: f64,
        y: f64,
        z: f64,
    ) -> f64 {
        let mut total_error = 0.0;
        let mut count = 0;

        for ((i, j), measured_tdoa) in tdoa_values {
            let dist_i = Self::dist(&devices[*i], x, y, z);
            let dist_j = Self::dist(&devices[*j], x, y, z);
            let predicted_tdoa = (dist_i - dist_j) / self.sound_speed;
            let error = (predicted_tdoa - measured_tdoa).powi(2);
            total_error += error;
            count += 1;
        }

        if count == 0 { 0.0 } else { total_error / count as f64 }
    }

    fn dist(device: &UrnDevice, x: f64, y: f64, z: f64) -> f64 {
        let dx = device.deployment_x - x;
        let dy = device.deployment_y - y;
        let dz = device.deployment_z - z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_devices() -> Vec<UrnDevice> {
        vec![
            UrnDevice { device_id: 1, device_name: "U1".to_string(), deployment_x: 0.0, deployment_y: 0.0, deployment_z: 0.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
            UrnDevice { device_id: 2, device_name: "U2".to_string(), deployment_x: 10.0, deployment_y: 0.0, deployment_z: 0.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
            UrnDevice { device_id: 3, device_name: "U3".to_string(), deployment_x: 5.0, deployment_y: 10.0, deployment_z: 0.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
        ]
    }

    #[test]
    fn test_beamformer_creation() {
        let bf = Beamformer::new(1500.0, 1.0, 500.0, 0.5);
        assert_eq!(bf.sound_speed, 1500.0);
    }

    #[test]
    fn test_mvdr_method() {
        let bf = Beamformer::new(1500.0, 10.0, 500.0, 0.3)
            .with_method(BeamformingMethod::MVDR);
        assert_eq!(bf.method, BeamformingMethod::MVDR);
    }

    #[test]
    fn test_covariance_matrix() {
        let devices = create_test_devices();
        let readings: Vec<(SensorReading, UrnDevice)> = devices.iter().map(|d| {
            (SensorReading {
                timestamp: Utc::now(),
                device_id: d.device_id,
                sound_pressure_level: 80.0,
                resonance_frequency: 200.0,
                source_direction: 45.0,
                medium_density: 1800.0,
                temperature: 20.0,
                humidity: 50.0,
            }, d.clone())
        }).collect();

        let bf = Beamformer::new(1500.0, 1.0, 500.0, 0.5);
        let cov = bf.estimate_covariance_matrix(&readings);
        assert_eq!(cov.len(), 3);
        assert_eq!(cov[0].len(), 3);
    }
}
