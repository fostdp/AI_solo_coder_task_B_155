(function (global) {
    'use strict';

    let scene, camera, renderer, controls;
    let urnMeshes = [];
    let sourceMarker = null;
    let devices = [];

    function initThree(containerId) {
        const container = document.getElementById(containerId);
        const width = container.clientWidth;
        const height = container.clientHeight;

        scene = new THREE.Scene();
        scene.background = new THREE.Color(0x050810);
        scene.fog = new THREE.FogExp2(0x050810, 0.002);

        camera = new THREE.PerspectiveCamera(60, width / height, 0.1, 2000);
        camera.position.set(150, 120, 150);

        renderer = new THREE.WebGLRenderer({ antialias: true });
        renderer.setSize(width, height);
        renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
        renderer.shadowMap.enabled = true;
        renderer.shadowMap.type = THREE.PCFSoftShadowMap;
        container.appendChild(renderer.domElement);

        controls = new THREE.OrbitControls(camera, renderer.domElement);
        controls.enableDamping = true;
        controls.dampingFactor = 0.08;
        controls.maxPolarAngle = Math.PI / 2.1;
        controls.minDistance = 50;
        controls.maxDistance = 500;

        setupLights();
        setupGround();
        setupGrid();

        window.addEventListener('resize', onWindowResize.bind(null, containerId));
    }

    function setupLights() {
        const ambient = new THREE.AmbientLight(0x404060, 0.6);
        scene.add(ambient);

        const dirLight = new THREE.DirectionalLight(0xffffff, 0.8);
        dirLight.position.set(100, 150, 80);
        dirLight.castShadow = true;
        dirLight.shadow.mapSize.width = 2048;
        dirLight.shadow.mapSize.height = 2048;
        dirLight.shadow.camera.left = -200;
        dirLight.shadow.camera.right = 200;
        dirLight.shadow.camera.top = 200;
        dirLight.shadow.camera.bottom = -200;
        scene.add(dirLight);

        const pointLight = new THREE.PointLight(0x6080ff, 0.5, 300);
        pointLight.position.set(0, 50, 0);
        scene.add(pointLight);
    }

    function setupGround() {
        const groundGeo = new THREE.PlaneGeometry(600, 600, 50, 50);
        const groundMat = new THREE.MeshStandardMaterial({
            color: 0x2a3520,
            roughness: 0.95,
            metalness: 0.0,
        });
        const ground = new THREE.Mesh(groundGeo, groundMat);
        ground.rotation.x = -Math.PI / 2;
        ground.position.y = -5;
        ground.receiveShadow = true;
        scene.add(ground);

        const soilGeo = new THREE.BoxGeometry(600, 20, 600);
        const soilMat = new THREE.MeshStandardMaterial({
            color: 0x3a2818,
            roughness: 1.0,
        });
        const soil = new THREE.Mesh(soilGeo, soilMat);
        soil.position.y = -15;
        scene.add(soil);
    }

    function setupGrid() {
        const grid = new THREE.GridHelper(400, 40, 0x2a3250, 0x151a2c);
        grid.position.y = -4.9;
        scene.add(grid);

        const axes = new THREE.AxesHelper(30);
        axes.position.y = -4.5;
        scene.add(axes);
    }

    function createUrnMesh(device) {
        const group = new THREE.Group();

        const urnBodyGeo = new THREE.SphereGeometry(8, 32, 24, 0, Math.PI * 2, 0, Math.PI * 0.65);
        const urnMat = new THREE.MeshStandardMaterial({
            color: 0x8b6914,
            roughness: 0.7,
            metalness: 0.2,
        });
        const urnBody = new THREE.Mesh(urnBodyGeo, urnMat);
        urnBody.position.y = -2;
        urnBody.castShadow = true;
        urnBody.receiveShadow = true;
        group.add(urnBody);

        const rimGeo = new THREE.TorusGeometry(3, 0.5, 16, 32);
        const rim = new THREE.Mesh(rimGeo, urnMat);
        rim.rotation.x = Math.PI / 2;
        rim.position.y = 4;
        group.add(rim);

        const neckGeo = new THREE.CylinderGeometry(2.5, 3, 4, 24);
        const neck = new THREE.Mesh(neckGeo, urnMat);
        neck.position.y = 2;
        group.add(neck);

        const innerGeo = new THREE.CircleGeometry(2.3, 24);
        const innerMat = new THREE.MeshStandardMaterial({
            color: 0x1a1208,
            roughness: 1.0,
            side: THREE.DoubleSide,
        });
        const inner = new THREE.Mesh(innerGeo, innerMat);
        inner.rotation.x = -Math.PI / 2;
        inner.position.y = 4.1;
        group.add(inner);

        const glowGeo = new THREE.RingGeometry(2.3, 3, 32);
        const glowMat = new THREE.MeshBasicMaterial({
            color: 0x3b82f6,
            transparent: true,
            opacity: 0.3,
            side: THREE.DoubleSide,
        });
        const glow = new THREE.Mesh(glowGeo, glowMat);
        glow.rotation.x = -Math.PI / 2;
        glow.position.y = 4.15;
        glow.name = 'glow';
        group.add(glow);

        const labelCanvas = document.createElement('canvas');
        labelCanvas.width = 256;
        labelCanvas.height = 64;
        const ctx = labelCanvas.getContext('2d');
        ctx.fillStyle = 'rgba(10, 15, 30, 0.85)';
        ctx.fillRect(0, 0, 256, 64);
        ctx.strokeStyle = '#3b82f6';
        ctx.lineWidth = 2;
        ctx.strokeRect(1, 1, 254, 62);
        ctx.fillStyle = '#a0c4ff';
        ctx.font = 'bold 24px Microsoft YaHei';
        ctx.textAlign = 'center';
        ctx.fillText(device.device_name, 128, 42);

        const labelTex = new THREE.CanvasTexture(labelCanvas);
        const labelMat = new THREE.SpriteMaterial({ map: labelTex, transparent: true });
        const label = new THREE.Sprite(labelMat);
        label.scale.set(25, 6, 1);
        label.position.y = 14;
        group.add(label);

        group.position.set(device.deployment_x, device.deployment_z + 5, device.deployment_y);
        group.userData = { deviceId: device.device_id, device };

        return group;
    }

    function createSourceMarker() {
        const group = new THREE.Group();

        const coreGeo = new THREE.SphereGeometry(4, 16, 16);
        const coreMat = new THREE.MeshBasicMaterial({
            color: 0xff3030,
            transparent: true,
            opacity: 0.9,
        });
        const core = new THREE.Mesh(coreGeo, coreMat);
        group.add(core);

        const ringGeo = new THREE.TorusGeometry(6, 0.3, 8, 32);
        const ringMat = new THREE.MeshBasicMaterial({
            color: 0xff6060,
            transparent: true,
            opacity: 0.6,
        });
        const ring = new THREE.Mesh(ringGeo, ringMat);
        ring.rotation.x = Math.PI / 2;
        ring.name = 'pulseRing';
        group.add(ring);

        const beamGeo = new THREE.ConeGeometry(2, 30, 8);
        const beamMat = new THREE.MeshBasicMaterial({
            color: 0xff2020,
            transparent: true,
            opacity: 0.3,
        });
        const beam = new THREE.Mesh(beamGeo, beamMat);
        beam.position.y = 15;
        group.add(beam);

        group.visible = false;
        return group;
    }

    function updateDevices(newDevices) {
        devices = newDevices;
        urnMeshes.forEach(m => scene.remove(m));
        urnMeshes = [];

        devices.forEach(device => {
            const mesh = createUrnMesh(device);
            scene.add(mesh);
            urnMeshes.push(mesh);
        });

        if (!sourceMarker) {
            sourceMarker = createSourceMarker();
            scene.add(sourceMarker);
        }
    }

    function updateDeviceGlow(deviceId, spl) {
        const mesh = urnMeshes.find(m => m.userData.deviceId === deviceId);
        if (!mesh) return;
        const glow = mesh.getObjectByName('glow');
        if (glow) {
            const intensity = Math.min(1, (spl - 60) / 60);
            glow.material.opacity = 0.2 + intensity * 0.6;
            const hue = 0.6 - intensity * 0.6;
            glow.material.color.setHSL(hue, 0.8, 0.5);
        }
    }

    function updateSourceMarker(loc) {
        if (!sourceMarker) return;
        sourceMarker.visible = true;
        sourceMarker.position.set(loc.source_x, Math.abs(loc.source_z) + 2, loc.source_y);
    }

    function resetView() {
        camera.position.set(150, 120, 150);
        controls.target.set(0, 0, 0);
        controls.update();
    }

    function render_3d() {
        const time = Date.now() * 0.001;
        urnMeshes.forEach(m => {
            const glow = m.getObjectByName('glow');
            if (glow) {
                const pulse = 1 + Math.sin(time * 2 + m.userData.deviceId) * 0.15;
                glow.scale.set(pulse, pulse, 1);
            }
        });

        if (sourceMarker && sourceMarker.visible) {
            const ring = sourceMarker.getObjectByName('pulseRing');
            if (ring) {
                const s = 1 + (time % 2) * 0.8;
                ring.scale.set(s, s, s);
                ring.material.opacity = Math.max(0, 0.8 - (time % 2) * 0.4);
            }
        }

        renderer.render(scene, camera);
    }

    function onWindowResize(containerId) {
        const container = document.getElementById(containerId);
        camera.aspect = container.clientWidth / container.clientHeight;
        camera.updateProjectionMatrix();
        renderer.setSize(container.clientWidth, container.clientHeight);
    }

    global.GroundListening3D = {
        init: initThree,
        updateDevices: updateDevices,
        updateDeviceGlow: updateDeviceGlow,
        updateSourceMarker: updateSourceMarker,
        resetView: resetView,
        render: render_3d,
        getDevices: function () { return devices; },
    };
})(window);
