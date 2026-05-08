import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
// ================================================================
// CONSTANTS
// ================================================================
const ARC_SEGMENTS = 64;
const ARC_WIDTH = 0.15;
const THRESHOLD_FRACTION = 2 / 3;
const COLOR_ROSE = new THREE.Color(0xaa7088); // --rd-rose
const COLOR_WARNING = new THREE.Color(0xc89a68); // --rd-warning
export function ThresholdArc({ radius, sceneStateRef }) {
    const arcRef = useRef(null);
    const animatedProgress = useRef(0);
    // Build the arc shape as a ring segment
    const arcGeometry = useMemo(() => {
        return new THREE.RingGeometry(radius - ARC_WIDTH, radius + ARC_WIDTH, ARC_SEGMENTS, 1, 0, Math.PI * 2);
    }, [radius]);
    // 2/3 threshold marker: use lineSegments (not <line> which maps to SVG)
    const thresholdAngle = THRESHOLD_FRACTION * Math.PI * 2;
    const thresholdGeometry = useMemo(() => {
        const inner = radius - ARC_WIDTH * 2;
        const outer = radius + ARC_WIDTH * 2;
        const geo = new THREE.BufferGeometry().setFromPoints([
            new THREE.Vector3(Math.cos(thresholdAngle) * inner, 0.01, Math.sin(thresholdAngle) * inner),
            new THREE.Vector3(Math.cos(thresholdAngle) * outer, 0.01, Math.sin(thresholdAngle) * outer),
        ]);
        return geo;
    }, [radius, thresholdAngle]);
    useFrame((_state, delta) => {
        const arc = arcRef.current;
        if (!arc)
            return;
        const { arcProgress, phase } = sceneStateRef.current;
        // Smooth the arc progress
        animatedProgress.current = THREE.MathUtils.lerp(animatedProgress.current, arcProgress, 6 * delta);
        // Update arc geometry: replace with a partial ring
        const progress = Math.max(0, Math.min(1, animatedProgress.current));
        const thetaLength = progress * Math.PI * 2;
        const newGeo = new THREE.RingGeometry(radius - ARC_WIDTH, radius + ARC_WIDTH, ARC_SEGMENTS, 1, -Math.PI / 2, // start from top
        thetaLength);
        arc.geometry.dispose();
        arc.geometry = newGeo;
        // Color: rose normally, amber during nullification
        const mat = arc.material;
        const isNullifying = phase === 'nullifying' || phase === 'nullified';
        const targetColor = isNullifying ? COLOR_WARNING : COLOR_ROSE;
        mat.color.lerp(targetColor, 6 * delta);
        // Opacity based on progress
        mat.opacity = THREE.MathUtils.lerp(mat.opacity, progress > 0.01 ? 0.6 : 0, 8 * delta);
    });
    return (_jsxs("group", { rotation: [-Math.PI / 2, 0, 0], position: [0, 0.01, 0], children: [_jsx("mesh", { ref: arcRef, geometry: arcGeometry, children: _jsx("meshBasicMaterial", { color: COLOR_ROSE, transparent: true, opacity: 0, side: THREE.DoubleSide, depthWrite: false, blending: THREE.AdditiveBlending }) }), _jsx("lineSegments", { geometry: thresholdGeometry, children: _jsx("lineBasicMaterial", { color: "#6a5a68", transparent: true, opacity: 0.3, depthWrite: false }) })] }));
}
