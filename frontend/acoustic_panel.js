(function (global) {
    'use strict';

    const API_BASE = location.protocol + '//' + (location.hostname || 'localhost') + ':' + (location.port || '8080');
    const WS_URL = (location.protocol === 'https:' ? 'wss:' : 'ws:') + '//' + (location.hostname || 'localhost') + ':' + (location.port || '8080') + '/ws';

    let waveAnimationRunning = true;
    let waves = [];
    let selectedDeviceId = null;

    const splHistory = [];
    const MAX_SPL_HISTORY = 60;

    let latestSensorData = {};
    let latestResonance = null;
    let latestLocalization = null;
    let alerts = [];
    let ws = null;

    const mediumLayers = [
        { name: 'dry_sand', depth: 0, thickness: 5, density: 1600, soundSpeed: 300, attenuation: 0.5, color: '#c9a46a' },
        { name: 'wet_sand', depth: 5, thickness: 15, density: 1900, soundSpeed: 500, attenuation: 0.3, color: '#a07840' },
        { name: 'clay', depth: 20, thickness: 30, density: 2200, soundSpeed: 1800, attenuation: 0.15, color: '#6b4423' },
        { name: 'limestone', depth: 50, thickness: 50, density: 2500, soundSpeed: 3500, attenuation: 0.05, color: '#8b8b8b' },
    ];

    let rayPaths = [];
    let rayStats = { reflections: 0, refractions: 0 };

    function snellsLaw(angleIncidence, v1, v2) {
        const sinTheta2 = (v2 / v1) * Math.sin(angleIncidence);
        if (Math.abs(sinTheta2) > 1) return null;
        return Math.asin(sinTheta2);
    }

    function reflectionCoefficient(angleIncidence, rho1, rho2, v1, v2) {
        const transAngle = snellsLaw(angleIncidence, v1, v2);
        if (transAngle === null) return 1.0;
        const normalR = Math.abs((rho2 * v2 - rho1 * v1) / (rho2 * v2 + rho1 * v1));
        const angularFactor = Math.abs(Math.cos(angleIncidence) - Math.cos(transAngle))
            / Math.abs(Math.cos(angleIncidence) + Math.cos(transAngle));
        return normalR * 0.5 + angularFactor * 0.5;
    }

    function transmissionCoefficient(angleIncidence, rho1, rho2, v1, v2) {
        const r = reflectionCoefficient(angleIncidence, rho1, rho2, v1, v2);
        return Math.sqrt(1 - r * r);
    }

    function traceRay(startX, startDepth, angle, frequency, maxBounces) {
        const points = [];
        let x = startX;
        let y = startDepth;
        let theta = angle;
        let amplitude = 1.0;
        let layerIdx = 0;
        let bounces = 0;
        let reflections = 0;
        let refractions = 0;

        for (let idx = 0; idx < mediumLayers.length; idx++) {
            const layer = mediumLayers[idx];
            if (y >= layer.depth && y < layer.depth + layer.thickness) {
                layerIdx = idx;
                break;
            }
        }

        points.push({ x, y, amplitude, layerIndex: layerIdx, type: 'start' });

        const maxSteps = 150;
        for (let step = 0; step < maxSteps; step++) {
            const layer = mediumLayers[layerIdx];
            if (!layer) break;

            const dirX = Math.sin(theta);
            const dirY = Math.cos(theta);

            if (Math.abs(dirY) < 0.001) break;

            let hitDepth = null;
            let hitType = null;

            if (dirY > 0) {
                const bottomY = layer.depth + layer.thickness;
                if (bottomY <= 80) {
                    hitDepth = bottomY;
                    hitType = 'bottom';
                }
            } else {
                const topY = layer.depth;
                if (topY > 0 || layerIdx === 0) {
                    hitDepth = topY;
                    hitType = 'top';
                }
            }

            if (hitDepth === null) break;

            const t = (hitDepth - y) / dirY;
            if (t <= 0) break;

            const hitX = x + dirX * t;

            const dist = t;
            amplitude *= Math.exp(-layer.attenuation * dist);
            amplitude /= Math.max(1, dist * 0.02);

            if (amplitude < 0.001) {
                points.push({ x: hitX, y: hitDepth, amplitude, layerIndex: layerIdx, type: 'end' });
                break;
            }

            if (hitType === 'bottom' && layerIdx < mediumLayers.length - 1) {
                const nextLayer = mediumLayers[layerIdx + 1];
                const incidentAngle = theta;
                const r = reflectionCoefficient(incidentAngle, layer.density, nextLayer.density, layer.soundSpeed, nextLayer.soundSpeed);
                const tCoef = transmissionCoefficient(incidentAngle, layer.density, nextLayer.density, layer.soundSpeed, nextLayer.soundSpeed);

                const transAngle = snellsLaw(incidentAngle, layer.soundSpeed, nextLayer.soundSpeed);

                points.push({
                    x: hitX, y: hitDepth,
                    amplitude: amplitude * tCoef,
                    layerIndex: layerIdx,
                    type: 'refraction'
                });
                refractions++;

                if (bounces < maxBounces) {
                    const reflectAmp = amplitude * r;
                    if (reflectAmp > 0.01) {
                        bounces++;
                        reflections++;
                    }
                }

                if (transAngle !== null) {
                    theta = transAngle;
                    layerIdx++;
                    amplitude *= tCoef;
                    x = hitX;
                    y = hitDepth;
                } else {
                    theta = -theta;
                    bounces++;
                    reflections++;
                    x = hitX;
                    y = hitDepth;
                }
            } else if (hitType === 'top' && layerIdx > 0) {
                const upperLayer = mediumLayers[layerIdx - 1];
                const incidentAngle = -theta;
                const r = reflectionCoefficient(incidentAngle, layer.density, upperLayer.density, layer.soundSpeed, upperLayer.soundSpeed);
                const tCoef = transmissionCoefficient(incidentAngle, layer.density, upperLayer.density, layer.soundSpeed, upperLayer.soundSpeed);

                const transAngle = snellsLaw(incidentAngle, layer.soundSpeed, upperLayer.soundSpeed);

                points.push({
                    x: hitX, y: hitDepth,
                    amplitude: amplitude * tCoef,
                    layerIndex: layerIdx,
                    type: 'refraction-up'
                });
                refractions++;

                if (transAngle !== null) {
                    theta = -transAngle;
                    layerIdx--;
                    amplitude *= tCoef;
                    x = hitX;
                    y = hitDepth;
                } else {
                    theta = -theta;
                    bounces++;
                    reflections++;
                    x = hitX;
                    y = hitDepth;
                }
            } else if (hitType === 'top' && layerIdx === 0) {
                points.push({
                    x: hitX, y: hitDepth,
                    amplitude,
                    layerIndex: 0,
                    type: 'surface'
                });
                break;
            } else {
                break;
            }
        }

        return { points, reflections, refractions };
    }

    function computeRayPaths(sourceX, sourceDepth, frequency) {
        const paths = [];
        let totalReflections = 0;
        let totalRefractions = 0;
        const numAngles = 15;

        for (let i = 0; i <= numAngles; i++) {
            const angle = -Math.PI / 2.5 + (Math.PI / 2.5) * i / numAngles;
            const result = traceRay(sourceX, sourceDepth, angle, frequency, 4);
            if (result.points.length > 1) {
                paths.push(result.points);
                totalReflections += result.reflections;
                totalRefractions += result.refractions;
            }
        }

        return { paths, reflections: totalReflections, refractions: totalRefractions };
    }

    function drawRayTracing() {
        const canvas = document.getElementById('ray-tracing-canvas');
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const w = canvas.width;
        const h = canvas.height;

        ctx.clearRect(0, 0, w, h);

        const maxDepth = 80;
        const scaleY = (h - 30) / maxDepth;
        const offsetX = 10;
        const plotW = w - offsetX * 2;

        mediumLayers.forEach((layer, idx) => {
            const y = 20 + layer.depth * scaleY;
            const layerH = layer.thickness * scaleY;
            const grad = ctx.createLinearGradient(0, y, 0, y + layerH);
            grad.addColorStop(0, layer.color);
            grad.addColorStop(1, shadeColor(layer.color, -15));
            ctx.fillStyle = grad;
            ctx.fillRect(offsetX, y, plotW, layerH);

            if (idx > 0) {
                ctx.beginPath();
                ctx.moveTo(offsetX, y);
                ctx.lineTo(offsetX + plotW, y);
                ctx.strokeStyle = 'rgba(255,255,255,0.15)';
                ctx.lineWidth = 1;
                ctx.stroke();
            }

            ctx.fillStyle = 'rgba(255,255,255,0.5)';
            ctx.font = '9px sans-serif';
            ctx.textAlign = 'left';
            ctx.fillText(`${layer.depth}m`, 2, y + 10);
        });

        ctx.fillStyle = 'rgba(255,255,255,0.4)';
        ctx.fillRect(offsetX, 20, plotW, 1);
        ctx.fillStyle = 'rgba(255,255,255,0.6)';
        ctx.font = '10px sans-serif';
        ctx.fillText('地表', offsetX + 4, 16);

        let sourceX = offsetX + plotW * 0.5;
        let sourceY = 20 + 2 * scaleY;

        if (latestLocalization) {
            const distFactor = Math.min(1, latestLocalization.distance_estimate / 300);
            sourceX = offsetX + plotW * (0.2 + distFactor * 0.6);
            sourceY = 20 + Math.abs(latestLocalization.source_z) * scaleY;
        }

        rayPaths.forEach((path, pidx) => {
            if (path.length < 2) return;
            const colorIntensity = 1 - pidx / rayPaths.length * 0.5;

            ctx.beginPath();
            ctx.moveTo(sourceX, sourceY);

            for (let i = 0; i < path.length; i++) {
                const pt = path[i];
                const ptX = sourceX + (pt.x - (latestLocalization ? latestLocalization.source_x : 0)) * 0.8;
                const ptY = 20 + pt.y * scaleY;
                ctx.lineTo(ptX, ptY);
            }

            const alpha = 0.3 + colorIntensity * 0.4;
            ctx.strokeStyle = `rgba(255, 200, 100, ${alpha})`;
            ctx.lineWidth = 1.2;
            ctx.stroke();
        });

        ctx.beginPath();
        ctx.arc(sourceX, sourceY, 6, 0, Math.PI * 2);
        const pulse = 0.7 + 0.3 * Math.sin(Date.now() / 300);
        ctx.fillStyle = `rgba(255, 80, 80, ${pulse})`;
        ctx.fill();
        ctx.strokeStyle = '#ff6060';
        ctx.lineWidth = 2;
        ctx.stroke();

        ctx.fillStyle = 'rgba(255,255,255,0.7)';
        ctx.font = '10px sans-serif';
        ctx.textAlign = 'left';
        ctx.fillText('声源', sourceX + 10, sourceY + 4);

        const urnX = offsetX + plotW * 0.5;
        const urnY = 20 + 2 * scaleY;
        ctx.beginPath();
        ctx.arc(urnX, urnY, 4, 0, Math.PI * 2);
        ctx.fillStyle = '#60a5fa';
        ctx.fill();
        ctx.strokeStyle = '#fff';
        ctx.lineWidth = 1;
        ctx.stroke();

        ctx.fillStyle = 'rgba(255,255,255,0.7)';
        ctx.font = '10px sans-serif';
        ctx.textAlign = 'center';
        ctx.fillText('瓮听', urnX, urnY - 8);
    }

    function shadeColor(color, percent) {
        const num = parseInt(color.replace('#', ''), 16);
        const amt = Math.round(2.55 * percent);
        const R = (num >> 16) + amt;
        const G = (num >> 8 & 0x00FF) + amt;
        const B = (num & 0x0000FF) + amt;
        return '#' + (
            0x1000000 +
            (R < 255 ? (R < 1 ? 0 : R) : 255) * 0x10000 +
            (G < 255 ? (G < 1 ? 0 : G) : 255) * 0x100 +
            (B < 255 ? (B < 1 ? 0 : B) : 255)
        ).toString(16).slice(1);
    }

    function updateRayTracing() {
        if (!latestLocalization) {
            const result = computeRayPaths(50, 10, 200);
            rayPaths = result.paths;
            rayStats.reflections = result.reflections;
            rayStats.refractions = result.refractions;
        } else {
            const freq = latestResonance ? latestResonance.measured_resonance_freq : 200;
            const result = computeRayPaths(
                latestLocalization.source_x,
                Math.abs(latestLocalization.source_z),
                freq
            );
            rayPaths = result.paths;
            rayStats.reflections = result.reflections;
            rayStats.refractions = result.refractions;
        }

        const reflEl = document.getElementById('ray-reflections');
        if (reflEl) reflEl.textContent = rayStats.reflections;
        const refrEl = document.getElementById('ray-refractions');
        if (refrEl) refrEl.textContent = rayStats.refractions;

        drawRayTracing();
    }

    function drawWaveOverlay() {
        const canvas = document.getElementById('wave-overlay');
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const w = canvas.width = canvas.clientWidth;
        const h = canvas.height = canvas.clientHeight;
        ctx.clearRect(0, 0, w, h);

        if (!waveAnimationRunning) return;

        const cx = w / 2;
        const cy = h / 2;

        waves.forEach((wave, idx) => {
            wave.radius += wave.speed;
            wave.opacity -= wave.fadeSpeed;

            if (wave.opacity <= 0) {
                waves.splice(idx, 1);
                return;
            }

            const mainGrad = ctx.createRadialGradient(cx, cy, wave.radius * 0.85, cx, cy, wave.radius);
            mainGrad.addColorStop(0, `rgba(96, 165, 250, 0)`);
            mainGrad.addColorStop(0.7, `rgba(96, 165, 250, ${wave.opacity * 0.25})`);
            mainGrad.addColorStop(0.9, `rgba(96, 165, 250, ${wave.opacity})`);
            mainGrad.addColorStop(1, `rgba(96, 165, 250, 0)`);

            ctx.beginPath();
            ctx.arc(cx, cy, wave.radius, 0, Math.PI * 2);
            ctx.fillStyle = mainGrad;
            ctx.fill();

            ctx.beginPath();
            ctx.arc(cx, cy, wave.radius, 0, Math.PI * 2);
            ctx.strokeStyle = `rgba(160, 196, 255, ${wave.opacity})`;
            ctx.lineWidth = 2;
            ctx.stroke();

            if (wave.radius > 40 && !wave.hasSecondary) {
                wave.hasSecondary = true;
                wave.secondaryRadius = 0;
            }
        });

        waves.forEach(wave => {
            if (wave.hasSecondary && wave.secondaryRadius !== undefined) {
                wave.secondaryRadius += wave.speed * 0.75;

                if (wave.secondaryRadius > 0 && wave.opacity * 0.5 > 0.01) {
                    const reflectGrad = ctx.createRadialGradient(
                        cx, cy, wave.secondaryRadius * 0.9,
                        cx, cy, wave.secondaryRadius
                    );
                    reflectGrad.addColorStop(0, `rgba(255, 180, 100, 0)`);
                    reflectGrad.addColorStop(0.8, `rgba(255, 180, 100, ${wave.opacity * 0.15})`);
                    reflectGrad.addColorStop(1, `rgba(255, 180, 100, 0)`);

                    ctx.beginPath();
                    ctx.arc(cx, cy, wave.secondaryRadius, 0, Math.PI * 2);
                    ctx.fillStyle = reflectGrad;
                    ctx.fill();

                    ctx.beginPath();
                    ctx.arc(cx, cy, wave.secondaryRadius, 0, Math.PI * 2);
                    ctx.strokeStyle = `rgba(255, 200, 120, ${wave.opacity * 0.5})`;
                    ctx.lineWidth = 1;
                    ctx.setLineDash([6, 4]);
                    ctx.stroke();
                    ctx.setLineDash([]);
                }

                if (wave.secondaryRadius > 60 && !wave.hasTertiary) {
                    wave.hasTertiary = true;
                    wave.tertiaryRadius = 0;
                }
            }

            if (wave.hasTertiary && wave.tertiaryRadius !== undefined) {
                wave.tertiaryRadius += wave.speed * 0.55;

                if (wave.tertiaryRadius > 0 && wave.opacity * 0.25 > 0.01) {
                    ctx.beginPath();
                    ctx.arc(cx, cy, wave.tertiaryRadius, 0, Math.PI * 2);
                    ctx.strokeStyle = `rgba(200, 220, 180, ${wave.opacity * 0.3})`;
                    ctx.lineWidth = 1;
                    ctx.setLineDash([3, 6]);
                    ctx.stroke();
                    ctx.setLineDash([]);
                }
            }
        });

        if (latestLocalization && waves.length === 0) {
            const distFactor = Math.min(1, latestLocalization.distance_estimate / 300);
            const beamAngle = (latestLocalization.bearing_angle - 90) * Math.PI / 180;
            const spread = 0.3 * (1.2 - latestLocalization.confidence);
            const beamLen = 80 + distFactor * 150;

            const beamGrad = ctx.createRadialGradient(
                cx, cy, 0,
                cx + Math.cos(beamAngle) * beamLen,
                cy + Math.sin(beamAngle) * beamLen,
                beamLen * 0.3
            );
            beamGrad.addColorStop(0, 'rgba(255, 100, 100, 0.3)');
            beamGrad.addColorStop(1, 'rgba(255, 100, 100, 0)');

            ctx.save();
            ctx.translate(cx, cy);
            ctx.rotate(beamAngle);
            ctx.beginPath();
            ctx.moveTo(0, 0);
            ctx.arc(0, 0, beamLen, -spread, spread);
            ctx.closePath();
            ctx.fillStyle = beamGrad;
            ctx.fill();
            ctx.restore();
        }
    }

    function spawnWave() {
        waves.push({
            radius: 10,
            speed: 2.5,
            opacity: 0.8,
            fadeSpeed: 0.008,
            hasSecondary: false,
            hasTertiary: false,
        });
    }

    function drawCompass() {
        const canvas = document.getElementById('compass-canvas');
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const cx = canvas.width / 2;
        const cy = canvas.height / 2;
        const r = 85;

        ctx.clearRect(0, 0, canvas.width, canvas.height);

        ctx.beginPath();
        ctx.arc(cx, cy, r, 0, Math.PI * 2);
        const bgGrad = ctx.createRadialGradient(cx, cy, r * 0.2, cx, cy, r);
        bgGrad.addColorStop(0, 'rgba(30, 40, 70, 0.9)');
        bgGrad.addColorStop(1, 'rgba(15, 20, 35, 0.95)');
        ctx.fillStyle = bgGrad;
        ctx.fill();
        ctx.strokeStyle = '#3b82f6';
        ctx.lineWidth = 2;
        ctx.stroke();

        for (let i = 0; i < 360; i += 5) {
            const angle = (i - 90) * Math.PI / 180;
            const isMajor = i % 30 === 0;
            const innerR = isMajor ? r - 14 : r - 8;
            const x1 = cx + Math.cos(angle) * innerR;
            const y1 = cy + Math.sin(angle) * innerR;
            const x2 = cx + Math.cos(angle) * (r - 3);
            const y2 = cy + Math.sin(angle) * (r - 3);
            ctx.beginPath();
            ctx.moveTo(x1, y1);
            ctx.lineTo(x2, y2);
            ctx.strokeStyle = isMajor ? '#88aaff' : '#4466aa';
            ctx.lineWidth = isMajor ? 2 : 1;
            ctx.stroke();
        }

        const labels = [
            { text: '北', angle: -90, y: -r + 22 },
            { text: '东', angle: 0, x: r - 20 },
            { text: '南', angle: 90, y: r - 15 },
            { text: '西', angle: 180, x: -r + 20 },
        ];
        ctx.font = 'bold 13px Microsoft YaHei';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        labels.forEach(l => {
            ctx.fillStyle = l.angle === -90 ? '#ff6060' : '#88aaff';
            const ang = l.angle * Math.PI / 180;
            const x = cx + (l.x !== undefined ? l.x : Math.cos(ang) * (r - 22));
            const y = cy + (l.y !== undefined ? l.y : Math.sin(ang) * (r - 22));
            ctx.fillText(l.text, x, y);
        });

        if (latestLocalization) {
            const bearing = latestLocalization.bearing_angle * Math.PI / 180;
            const confidence = latestLocalization.confidence;

            ctx.beginPath();
            ctx.moveTo(cx, cy);
            const len = r - 20;
            const tipX = cx + Math.sin(bearing) * len;
            const tipY = cy - Math.cos(bearing) * len;
            ctx.lineTo(tipX, tipY);
            ctx.strokeStyle = confidence > 0.6 ? '#4ade80' : confidence > 0.3 ? '#fbbf24' : '#f87171';
            ctx.lineWidth = 4;
            ctx.lineCap = 'round';
            ctx.stroke();

            const headLen = 12;
            const headAng = 0.4;
            ctx.beginPath();
            ctx.moveTo(tipX, tipY);
            ctx.lineTo(
                tipX - headLen * Math.sin(bearing - headAng),
                tipY + headLen * Math.cos(bearing - headAng)
            );
            ctx.moveTo(tipX, tipY);
            ctx.lineTo(
                tipX - headLen * Math.sin(bearing + headAng),
                tipY + headLen * Math.cos(bearing + headAng)
            );
            ctx.stroke();

            const arcR = r - 8;
            const arcSpread = (1 - confidence) * Math.PI * 0.8 + 0.1;
            ctx.beginPath();
            ctx.arc(cx, cy, arcR, bearing - arcSpread / 2 - Math.PI / 2, bearing + arcSpread / 2 - Math.PI / 2);
            ctx.strokeStyle = `rgba(239, 68, 68, ${confidence * 0.7})`;
            ctx.lineWidth = 3;
            ctx.stroke();
        }

        ctx.beginPath();
        ctx.arc(cx, cy, 8, 0, Math.PI * 2);
        ctx.fillStyle = '#1e2a4a';
        ctx.fill();
        ctx.strokeStyle = '#3b82f6';
        ctx.lineWidth = 2;
        ctx.stroke();
    }

    function drawResonanceChart() {
        const canvas = document.getElementById('resonance-canvas');
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const w = canvas.width;
        const h = canvas.height;
        const padding = { top: 10, right: 10, bottom: 25, left: 35 };

        ctx.clearRect(0, 0, w, h);
        ctx.fillStyle = '#0d1220';
        ctx.fillRect(0, 0, w, h);

        const chartW = w - padding.left - padding.right;
        const chartH = h - padding.top - padding.bottom;

        ctx.strokeStyle = '#1c2336';
        ctx.lineWidth = 1;
        for (let i = 0; i <= 4; i++) {
            const y = padding.top + (chartH / 4) * i;
            ctx.beginPath();
            ctx.moveTo(padding.left, y);
            ctx.lineTo(w - padding.right, y);
            ctx.stroke();
        }

        if (!latestResonance) {
            ctx.fillStyle = '#6b7690';
            ctx.font = '12px Microsoft YaHei';
            ctx.textAlign = 'center';
            ctx.fillText('等待数据...', w / 2, h / 2);
            return;
        }

        const f0 = latestResonance.theoretical_resonance_freq;
        const freqMin = Math.max(0, f0 * 0.3);
        const freqMax = f0 * 1.8;
        const gainMin = -20;
        const gainMax = 40;
        const q = latestResonance.quality_factor || 20;

        ctx.beginPath();
        let firstPoint = true;
        for (let i = 0; i <= 100; i++) {
            const f = freqMin + (freqMax - freqMin) * (i / 100);
            const ratio = f / f0;
            const gain = 20 * Math.log10(q / Math.sqrt(Math.pow(1 - ratio * ratio, 2) + Math.pow(ratio / q, 2)));
            const px = padding.left + ((f - freqMin) / (freqMax - freqMin)) * chartW;
            const py = padding.top + chartH - ((gain - gainMin) / (gainMax - gainMin)) * chartH;
            if (firstPoint) {
                ctx.moveTo(px, py);
                firstPoint = false;
            } else {
                ctx.lineTo(px, py);
            }
        }

        const grad = ctx.createLinearGradient(0, padding.top, 0, padding.top + chartH);
        grad.addColorStop(0, 'rgba(96, 165, 250, 0.4)');
        grad.addColorStop(1, 'rgba(96, 165, 250, 0)');
        ctx.lineTo(w - padding.right, padding.top + chartH);
        ctx.lineTo(padding.left, padding.top + chartH);
        ctx.closePath();
        ctx.fillStyle = grad;
        ctx.fill();

        ctx.beginPath();
        firstPoint = true;
        for (let i = 0; i <= 100; i++) {
            const f = freqMin + (freqMax - freqMin) * (i / 100);
            const ratio = f / f0;
            const gain = 20 * Math.log10(q / Math.sqrt(Math.pow(1 - ratio * ratio, 2) + Math.pow(ratio / q, 2)));
            const px = padding.left + ((f - freqMin) / (freqMax - freqMin)) * chartW;
            const py = padding.top + chartH - ((gain - gainMin) / (gainMax - gainMin)) * chartH;
            if (firstPoint) {
                ctx.moveTo(px, py);
                firstPoint = false;
            } else {
                ctx.lineTo(px, py);
            }
        }
        ctx.strokeStyle = '#60a5fa';
        ctx.lineWidth = 2;
        ctx.stroke();

        const measuredX = padding.left + ((latestResonance.measured_resonance_freq - freqMin) / (freqMax - freqMin)) * chartW;
        ctx.beginPath();
        ctx.moveTo(measuredX, padding.top);
        ctx.lineTo(measuredX, padding.top + chartH);
        ctx.strokeStyle = 'rgba(251, 191, 36, 0.6)';
        ctx.lineWidth = 1;
        ctx.setLineDash([4, 4]);
        ctx.stroke();
        ctx.setLineDash([]);

        ctx.fillStyle = '#6b7690';
        ctx.font = '10px monospace';
        ctx.textAlign = 'center';
        ctx.fillText(`${Math.round(freqMin)}Hz`, padding.left, h - 8);
        ctx.fillText(`${Math.round(freqMax)}Hz`, w - padding.right, h - 8);
        ctx.fillText('频率', w / 2, h - 3);
        ctx.textAlign = 'right';
        ctx.fillText(`${gainMax}dB`, padding.left - 4, padding.top + 4);
        ctx.fillText(`${gainMin}dB`, padding.left - 4, padding.top + chartH);
    }

    function drawSparkline() {
        const canvas = document.getElementById('spl-canvas');
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const w = canvas.width;
        const h = canvas.height;

        ctx.clearRect(0, 0, w, h);

        if (splHistory.length < 2) {
            ctx.fillStyle = '#6b7690';
            ctx.font = '11px Microsoft YaHei';
            ctx.textAlign = 'center';
            ctx.fillText('等待声压级数据...', w / 2, h / 2);
            return;
        }

        const values = splHistory.slice(-MAX_SPL_HISTORY);
        const min = Math.min(...values) - 5;
        const max = Math.max(...values) + 5;

        ctx.beginPath();
        values.forEach((v, i) => {
            const x = (i / (values.length - 1)) * w;
            const y = h - ((v - min) / (max - min)) * (h - 4) - 2;
            if (i === 0) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
        });

        const grad = ctx.createLinearGradient(0, 0, 0, h);
        grad.addColorStop(0, 'rgba(251, 191, 36, 0.4)');
        grad.addColorStop(1, 'rgba(251, 191, 36, 0)');
        ctx.lineTo(w, h);
        ctx.lineTo(0, h);
        ctx.closePath();
        ctx.fillStyle = grad;
        ctx.fill();

        ctx.beginPath();
        values.forEach((v, i) => {
            const x = (i / (values.length - 1)) * w;
            const y = h - ((v - min) / (max - min)) * (h - 4) - 2;
            if (i === 0) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
        });
        ctx.strokeStyle = '#fbbf24';
        ctx.lineWidth = 1.5;
        ctx.stroke();
    }

    function updateUI() {
        const now = new Date();
        const timeEl = document.getElementById('update-time');
        if (timeEl) timeEl.textContent = now.toLocaleTimeString('zh-CN');

        if (latestResonance) {
            const freqEl = document.getElementById('current-freq');
            if (freqEl) freqEl.textContent = `${latestResonance.measured_resonance_freq.toFixed(1)} Hz`;
            const theoEl = document.getElementById('theoretical-freq');
            if (theoEl) theoEl.textContent = `${latestResonance.theoretical_resonance_freq.toFixed(1)} Hz`;
            const driftEl = document.getElementById('freq-drift');
            if (driftEl) {
                driftEl.textContent = `${latestResonance.drift_percent.toFixed(2)} %`;
                driftEl.className = latestResonance.drift_percent > 10 ? 'danger' : latestResonance.drift_percent > 5 ? 'warning' : 'success';
            }
            const gainEl = document.getElementById('gain-db');
            if (gainEl) gainEl.textContent = `${latestResonance.gain_db.toFixed(1)} dB`;
            const qEl = document.getElementById('quality-factor');
            if (qEl) qEl.textContent = latestResonance.quality_factor.toFixed(1);
        }

        if (latestLocalization) {
            const bearEl = document.getElementById('bearing-angle');
            if (bearEl) bearEl.textContent = `${latestLocalization.bearing_angle.toFixed(1)} °`;
            const elevEl = document.getElementById('elevation-angle');
            if (elevEl) elevEl.textContent = `${latestLocalization.elevation_angle.toFixed(1)} °`;
            const distEl = document.getElementById('distance-est');
            if (distEl) distEl.textContent = `${latestLocalization.distance_estimate.toFixed(0)} m`;
            const confEl = document.getElementById('confidence');
            if (confEl) {
                const confPct = latestLocalization.confidence * 100;
                confEl.textContent = `${confPct.toFixed(0)} %`;
                confEl.className = confPct > 60 ? 'success' : confPct > 30 ? 'warning' : 'danger';
            }
        }

        const sensorData = latestSensorData;
        if (Object.keys(sensorData).length > 0) {
            const keys = Object.keys(sensorData);
            const latest = sensorData[selectedDeviceId || keys[0]];
            if (latest) {
                const splEl = document.getElementById('spl-value');
                if (splEl) splEl.textContent = `${latest.sound_pressure_level.toFixed(1)} dB`;
                const denEl = document.getElementById('density-value');
                if (denEl) denEl.textContent = `${latest.medium_density.toFixed(0)} kg/m³`;
                const tempEl = document.getElementById('temp-value');
                if (tempEl) tempEl.textContent = `${latest.temperature.toFixed(1)} °C`;
                const humEl = document.getElementById('humidity-value');
                if (humEl) humEl.textContent = `${latest.humidity.toFixed(0)} %`;
            }
        }

        drawResonanceChart();
        drawCompass();
        drawSparkline();
        drawWaveOverlay();
        updateRayTracing();
    }

    function renderAlerts() {
        const list = document.getElementById('alert-list');
        if (!list) return;
        list.innerHTML = '';
        if (alerts.length === 0) {
            list.innerHTML = '<div style="color:#6b7690;font-size:12px;text-align:center;padding:16px;">暂无告警</div>';
            return;
        }
        alerts.slice(0, 20).forEach(alert => {
            const div = document.createElement('div');
            div.className = `alert-item ${alert.severity}`;
            const t = new Date(alert.timestamp);
            div.innerHTML = `
                <div class="alert-type">${alert.alert_type === 'frequency_drift' ? '频率漂移' : '定位偏差'} [${alert.severity === 'critical' ? '严重' : '警告'}]</div>
                <div class="alert-message">${alert.message}</div>
                <div class="alert-time">${t.toLocaleString('zh-CN')}</div>
            `;
            list.appendChild(div);
        });
    }

    function renderDeviceList() {
        const list = document.getElementById('device-list');
        if (!list) return;
        const devices = global.GroundListening3D ? global.GroundListening3D.getDevices() : [];
        list.innerHTML = '';
        devices.forEach(device => {
            const div = document.createElement('div');
            div.className = `device-item ${selectedDeviceId === device.device_id ? 'active' : ''}`;
            const data = latestSensorData[device.device_id];
            div.innerHTML = `
                <div class="device-item-name">${device.device_name}</div>
                <div class="device-item-meta">
                    ID: ${device.device_id} | 位置: (${device.deployment_x}, ${device.deployment_y})
                    ${data ? ` | SPL: ${data.sound_pressure_level.toFixed(0)}dB` : ''}
                </div>
            `;
            div.onclick = () => {
                selectedDeviceId = device.device_id;
                renderDeviceList();
                updateUI();
            };
            list.appendChild(div);
        });
    }

    function initUI() {
        const btnToggle = document.getElementById('btn-toggle-wave');
        if (btnToggle) {
            btnToggle.onclick = function () {
                waveAnimationRunning = !waveAnimationRunning;
                this.textContent = waveAnimationRunning ? '停止波纹动画' : '启动波纹动画';
            };
        }

        const btnReset = document.getElementById('btn-reset-view');
        if (btnReset) {
            btnReset.onclick = function () {
                if (global.GroundListening3D) {
                    global.GroundListening3D.resetView();
                }
            };
        }

        const btnSim = document.getElementById('btn-simulate-event');
        if (btnSim) {
            btnSim.onclick = function () {
                simulateEnemyEvent();
            };
        }
    }

    function connectWebSocket() {
        const statusEl = document.getElementById('connection-status');
        const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
        const host = location.hostname || 'localhost';
        const port = location.port || '8080';
        ws = new WebSocket(`${proto}//${host}:${port}/ws`);

        ws.onopen = () => {
            if (statusEl) {
                statusEl.textContent = '已连接';
                statusEl.className = 'status-badge connected';
            }
        };

        ws.onclose = () => {
            if (statusEl) {
                statusEl.textContent = '连接断开';
                statusEl.className = 'status-badge disconnected';
            }
            setTimeout(connectWebSocket, 3000);
        };

        ws.onerror = () => {
            if (statusEl) {
                statusEl.textContent = '连接错误';
                statusEl.className = 'status-badge disconnected';
            }
        };

        ws.onmessage = (event) => {
            try {
                const msg = JSON.parse(event.data);
                handleWsMessage(msg);
            } catch (e) {
                console.error('WS消息解析失败:', e);
            }
        };
    }

    function handleWsMessage(msg) {
        switch (msg.message_type) {
            case 'sensor_data':
                latestSensorData[msg.data.device_id] = msg.data;
                splHistory.push(msg.data.sound_pressure_level);
                if (splHistory.length > MAX_SPL_HISTORY) splHistory.shift();
                spawnWave();
                if (global.GroundListening3D) {
                    global.GroundListening3D.updateDeviceGlow(msg.data.device_id, msg.data.sound_pressure_level);
                }
                break;
            case 'resonance':
                latestResonance = msg.data;
                break;
            case 'localization':
                latestLocalization = msg.data;
                if (global.GroundListening3D) {
                    global.GroundListening3D.updateSourceMarker(msg.data);
                }
                break;
            case 'alert':
                alerts.unshift(msg.data);
                if (alerts.length > 50) alerts.pop();
                renderAlerts();
                break;
        }
        renderDeviceList();
    }

    async function fetchDevices() {
        try {
            const res = await fetch(`${API_BASE}/api/devices`);
            const json = await res.json();
            if (json.success) {
                if (global.GroundListening3D) {
                    global.GroundListening3D.updateDevices(json.data);
                }
                selectedDeviceId = json.data[0]?.device_id;
                renderDeviceList();
            }
        } catch (e) {
            console.warn('获取设备列表失败，使用默认设备:', e);
            useDefaultDevices();
        }
    }

    function useDefaultDevices() {
        const defaultDevices = [
            { device_id: 1, device_name: '瓮听-东北角', deployment_x: -50, deployment_y: -50, deployment_z: -2, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
            { device_id: 2, device_name: '瓮听-东南角', deployment_x: 50, deployment_y: -50, deployment_z: -2, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
            { device_id: 3, device_name: '瓮听-西南角', deployment_x: 50, deployment_y: 50, deployment_z: -2, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
            { device_id: 4, device_name: '瓮听-西北角', deployment_x: -50, deployment_y: 50, deployment_z: -2, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
            { device_id: 5, device_name: '瓮听-正中央', deployment_x: 0, deployment_y: 0, deployment_z: -2, urn_volume: 0.08, neck_radius: 0.06, neck_length: 0.12 },
        ];
        if (global.GroundListening3D) {
            global.GroundListening3D.updateDevices(defaultDevices);
        }
        selectedDeviceId = 1;
        renderDeviceList();
    }

    async function fetchAlerts() {
        try {
            const res = await fetch(`${API_BASE}/api/alerts?limit=20`);
            const json = await res.json();
            if (json.success) {
                alerts = json.data || [];
                renderAlerts();
            }
        } catch (e) {
            console.warn('获取告警列表失败:', e);
        }
    }

    function simulateEnemyEvent() {
        const angle = Math.random() * Math.PI * 2;
        const distance = 150 + Math.random() * 200;
        const x = Math.cos(angle) * distance;
        const y = Math.sin(angle) * distance;

        const bearing = (angle * 180 / Math.PI + 450) % 360;

        latestLocalization = {
            source_id: Date.now(),
            source_x: x,
            source_y: y,
            source_z: -3,
            bearing_angle: bearing,
            elevation_angle: -5,
            distance_estimate: distance,
            confidence: 0.5 + Math.random() * 0.4,
        };
        if (global.GroundListening3D) {
            global.GroundListening3D.updateSourceMarker(latestLocalization);
        }

        const devices = global.GroundListening3D ? global.GroundListening3D.getDevices() : [];
        devices.forEach(d => {
            const ddx = d.deployment_x - x;
            const ddy = d.deployment_y - y;
            const dist = Math.sqrt(ddx * ddx + ddy * ddy);
            const baseSpl = 100 - dist * 0.2;
            const reading = {
                device_id: d.device_id,
                sound_pressure_level: Math.max(60, baseSpl + (Math.random() - 0.5) * 10),
                resonance_frequency: 180 + (Math.random() - 0.5) * 30,
                source_direction: bearing,
                medium_density: 1800 + Math.random() * 300,
                temperature: 18 + Math.random() * 5,
                humidity: 45 + Math.random() * 20,
                timestamp: new Date().toISOString(),
            };
            latestSensorData[d.device_id] = reading;
            splHistory.push(reading.sound_pressure_level);
            if (splHistory.length > MAX_SPL_HISTORY) splHistory.shift();
            spawnWave();
            if (global.GroundListening3D) {
                global.GroundListening3D.updateDeviceGlow(d.device_id, reading.sound_pressure_level);
            }
        });

        latestResonance = {
            device_id: 1,
            measured_resonance_freq: 195,
            theoretical_resonance_freq: 185,
            gain_db: 28,
            quality_factor: 25,
            frequency_drift: 10,
            drift_percent: 5.4,
            is_anomaly: true,
            timestamp: new Date().toISOString(),
        };

        renderDeviceList();
    }

    function animate() {
        requestAnimationFrame(animate);
        if (global.GroundListening3D) {
            global.GroundListening3D.render();
        }
        updateUI();
    }

    function init() {
        if (global.GroundListening3D) {
            global.GroundListening3D.init('three-container');
        }
        initUI();
        fetchDevices();
        fetchAlerts();
        connectWebSocket();
        animate();
    }

    global.AcousticPanel = {
        init: init,
        getLatestSensorData: () => latestSensorData,
        getLatestResonance: () => latestResonance,
        getLatestLocalization: () => latestLocalization,
        getAlerts: () => alerts,
    };

    document.addEventListener('DOMContentLoaded', init);
})(window);
