import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
// ================================================================
// COLORS & OPACITY PER STATE
// ================================================================
const CENTER_COLORS = {
    empty: new THREE.Color(0x3a303a), // --rd-text-ghost
    wireframe: new THREE.Color(0x5a5a78), // --rd-dream-dim
    filling: new THREE.Color(0x7a7a98), // --rd-dream
    solid: new THREE.Color(0x9494b4), // --rd-dream-bright
    crystallized: new THREE.Color(0xd8c8a0), // --rd-bone-bright
    shattering: new THREE.Color(0xcc5555), // --rd-danger
};
const CENTER_OPACITY = {
    empty: 0.05,
    wireframe: 0.2,
    filling: 0.4,
    solid: 0.7,
    crystallized: 1.0,
    shattering: 0.6,
};
const CENTER_SCALE = {
    empty: 0.3,
    wireframe: 0.6,
    filling: 0.8,
    solid: 1.0,
    crystallized: 1.0,
    shattering: 0.3,
};
const _edgesGeo = new THREE.EdgesGeometry(new THREE.BoxGeometry(1.2, 1.2, 1.2));
export function CenterBlock({ sceneStateRef }) {
    const meshRef = useRef(null);
    const wireRef = useRef(null);
    const flashRef = useRef(null);
    // Fragment meshes for shatter animation
    const fragmentRefs = useRef(Array.from({ length: 8 }, () => null));
    const fragmentVelocities = useRef(Array.from({ length: 8 }, () => new THREE.Vector3((Math.random() - 0.5) * 4, Math.random() * 2, (Math.random() - 0.5) * 4)));
    const shatterStartRef = useRef(0);
    const prevCenterState = useRef('empty');
    useFrame((_state, delta) => {
        const mesh = meshRef.current;
        const wire = wireRef.current;
        const flash = flashRef.current;
        if (!mesh || !wire)
            return;
        const { centerState } = sceneStateRef.current;
        const targetColor = CENTER_COLORS[centerState];
        const targetOpacity = CENTER_OPACITY[centerState];
        const targetScale = CENTER_SCALE[centerState];
        // Detect transition to crystallized for flash trigger
        if (centerState === 'crystallized' &&
            prevCenterState.current !== 'crystallized') {
            if (flash) {
                flash.material.opacity = 0.8;
            }
        }
        prevCenterState.current = centerState;
        // Smooth color transition
        const mat = mesh.material;
        mat.color.lerp(targetColor, 6 * delta);
        mat.opacity = THREE.MathUtils.lerp(mat.opacity, targetOpacity, 6 * delta);
        mat.emissive.lerp(targetColor, 4 * delta);
        mat.emissiveIntensity = THREE.MathUtils.lerp(mat.emissiveIntensity, centerState === 'crystallized' ? 0.8 : 0.2, 4 * delta);
        // Wireframe visibility: show for wireframe + filling states
        const wireMat = wire.material;
        const wireVisible = centerState === 'wireframe' || centerState === 'filling';
        wireMat.opacity = THREE.MathUtils.lerp(wireMat.opacity, wireVisible ? 0.4 : 0, 6 * delta);
        // Scale
        const currentScale = mesh.scale.x;
        mesh.scale.setScalar(THREE.MathUtils.lerp(currentScale, targetScale, 6 * delta));
        wire.scale.copy(mesh.scale);
        // Gentle rotation
        mesh.rotation.y += delta * 0.3;
        wire.rotation.y = mesh.rotation.y;
        // Crystallized flash fade-out
        if (flash) {
            const flashMat = flash.material;
            if (flashMat.opacity > 0) {
                flashMat.opacity = Math.max(0, flashMat.opacity - delta * 5);
            }
            flash.rotation.y = mesh.rotation.y;
            flash.scale.copy(mesh.scale);
        }
        // Shatter animation
        if (centerState === 'shattering') {
            if (shatterStartRef.current === 0) {
                shatterStartRef.current = Date.now();
            }
            const elapsed = (Date.now() - shatterStartRef.current) / 1000;
            for (let i = 0; i < fragmentRefs.current.length; i++) {
                const frag = fragmentRefs.current[i];
                if (!frag)
                    continue;
                const vel = fragmentVelocities.current[i];
                frag.position.add(vel.clone().multiplyScalar(delta));
                frag.material.opacity = Math.max(0, 1 - elapsed * 2);
                frag.rotation.x += delta * 3;
                frag.rotation.z += delta * 2;
            }
        }
        else {
            shatterStartRef.current = 0;
            // Reset fragments to center
            for (const frag of fragmentRefs.current) {
                if (frag) {
                    frag.position.set(0, 0, 0);
                    frag.material.opacity = 0;
                }
            }
        }
    });
    return (_jsxs("group", { position: [0, 0, 0], children: [_jsxs("mesh", { ref: meshRef, children: [_jsx("boxGeometry", { args: [1.2, 1.2, 1.2] }), _jsx("meshStandardMaterial", { color: CENTER_COLORS.empty, emissive: CENTER_COLORS.empty, emissiveIntensity: 0.2, transparent: true, opacity: 0.05, roughness: 0.2, metalness: 0.7 })] }), _jsx("lineSegments", { ref: wireRef, geometry: _edgesGeo, children: _jsx("lineBasicMaterial", { color: "#9494b4", transparent: true, opacity: 0, depthWrite: false }) }), _jsxs("mesh", { ref: flashRef, children: [_jsx("boxGeometry", { args: [1.3, 1.3, 1.3] }), _jsx("meshBasicMaterial", { color: "#ffffff", transparent: true, opacity: 0, depthWrite: false, blending: THREE.AdditiveBlending })] }), Array.from({ length: 8 }, (_, i) => (_jsxs("mesh", { ref: (el) => {
                    fragmentRefs.current[i] = el;
                }, children: [_jsx("boxGeometry", { args: [0.3, 0.3, 0.3] }), _jsx("meshStandardMaterial", { color: "#cc5555", transparent: true, opacity: 0, emissive: "#cc5555", emissiveIntensity: 0.5 })] }, i)))] }));
}
