import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { PerspectiveCamera } from '@react-three/drei';
import { useRef, useEffect, useMemo, Suspense } from 'react';
import * as THREE from 'three';
import { bus } from '@/data/bus';
import { TerrainRing } from './TerrainRing';
import { TerrainTileMesh } from './TerrainTile';
import { TransactionBeams } from './beams';
const FOG_COLOR = new THREE.Color(0x060608);
const AMBIENT_COLOR = new THREE.Color(0x3a2030);
/** Manages the camera tilt driven by pointer position. */
function CameraController() {
    const { camera } = useThree();
    const targetRotX = useRef(0);
    const targetRotY = useRef(0);
    const currentRotX = useRef(0);
    const currentRotY = useRef(0);
    useEffect(() => {
        const onPointerMove = (e) => {
            const nx = (e.clientX / window.innerWidth) * 2 - 1;
            const ny = (e.clientY / window.innerHeight) * 2 - 1;
            const MAX_TILT = (5 * Math.PI) / 180;
            targetRotX.current = -ny * MAX_TILT;
            targetRotY.current = nx * MAX_TILT;
        };
        window.addEventListener('pointermove', onPointerMove, { passive: true });
        return () => window.removeEventListener('pointermove', onPointerMove);
    }, []);
    useFrame(() => {
        const LERP = 0.04;
        currentRotX.current += (targetRotX.current - currentRotX.current) * LERP;
        currentRotY.current += (targetRotY.current - currentRotY.current) * LERP;
        camera.rotation.x = currentRotX.current + (camera.userData.baseRotX ?? 0);
        camera.rotation.y = currentRotY.current;
    });
    return null;
}
/** Inner scene content rendered within the R3F Canvas. */
function TerrainSceneContent() {
    const ring = useMemo(() => new TerrainRing(10), []);
    const groupRef = useRef(null);
    // Subscribe to new blocks imperatively -- no React re-renders
    useEffect(() => {
        const handler = (block) => {
            ring.advance(block);
        };
        bus.on('block:new', handler);
        return () => {
            bus.off('block:new', handler);
        };
    }, [ring]);
    // Animate the scroll: tiles move toward camera at a constant speed
    useFrame((_, delta) => {
        ring.update(delta);
    });
    return (_jsxs(_Fragment, { children: [_jsx(PerspectiveCamera, { makeDefault: true, position: [0, 8, 12], fov: 15, near: 0.1, far: 100, onUpdate: (cam) => {
                    cam.lookAt(0, 0, -4);
                    cam.userData.baseRotX = cam.rotation.x;
                } }), _jsx("fogExp2", { attach: "fog", args: [FOG_COLOR, 0.025] }), _jsx("ambientLight", { color: AMBIENT_COLOR, intensity: 0.15 }), _jsx("directionalLight", { position: [-4, 8, 4], intensity: 0.7, color: 0xffeedd }), _jsx(CameraController, {}), _jsx("group", { ref: groupRef, children: ring.getSlots().map((slot) => (_jsx(TerrainTileMesh, { slot: slot }, slot.id))) }), _jsx(TransactionBeams, { ring: ring })] }));
}
/** Top-level exported component. Lazy-loaded by SceneManager. */
export default function TerrainScene() {
    return (_jsx(Canvas, { gl: {
            antialias: true,
            alpha: true,
            powerPreference: 'high-performance',
        }, style: { position: 'absolute', inset: 0 }, dpr: [1, 2], children: _jsx(Suspense, { fallback: null, children: _jsx(TerrainSceneContent, {}) }) }));
}
