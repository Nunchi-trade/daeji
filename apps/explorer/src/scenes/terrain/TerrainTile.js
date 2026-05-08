import { jsx as _jsx } from "react/jsx-runtime";
import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import vertexShader from './terrain.vert.glsl';
import fragmentShader from './terrain.frag.glsl';
const TILE_SIZE = 8;
const GRID_RES = 31; // segments per axis (32 vertices)
const GRID_VERTS = 32;
function createHeightTexture(heightmap) {
    const tex = new THREE.DataTexture(heightmap, GRID_VERTS, GRID_VERTS, THREE.RedFormat, THREE.FloatType);
    tex.minFilter = THREE.LinearFilter;
    tex.magFilter = THREE.LinearFilter;
    tex.wrapS = THREE.ClampToEdgeWrapping;
    tex.wrapT = THREE.ClampToEdgeWrapping;
    tex.needsUpdate = true;
    return tex;
}
export function TerrainTileMesh({ slot }) {
    const meshRef = useRef(null);
    const currentHashRef = useRef(null);
    const texRef = useRef(null);
    const geometry = useMemo(() => {
        const geo = new THREE.PlaneGeometry(TILE_SIZE, TILE_SIZE, GRID_RES, GRID_RES);
        geo.rotateX(-Math.PI / 2);
        return geo;
    }, []);
    const material = useMemo(() => {
        return new THREE.ShaderMaterial({
            vertexShader,
            fragmentShader,
            uniforms: {
                uHeightmap: { value: null },
                uActivityLevel: { value: 0 },
                uHueShift: { value: 0.5 },
                uSaturation: { value: 0.5 },
                uTime: { value: 0 },
                uOpacity: { value: 0 },
                // ROSEDUST palette
                uColorDeep: { value: new THREE.Color(0x0a0f1a) },
                uColorMid: { value: new THREE.Color(0x3a2030) },
                uColorHigh: { value: new THREE.Color(0xdca5bd) },
                uColorPeak: { value: new THREE.Color(0xd8c8a0) },
                // Fog
                uFogColor: { value: new THREE.Color(0x060608) },
                uFogDensity: { value: 0.025 },
            },
            transparent: true,
            side: THREE.DoubleSide,
        });
    }, []);
    // Imperatively check for slot data changes every frame
    // (slots are mutated in place by TerrainRing, bypassing React)
    useFrame((_, delta) => {
        if (!meshRef.current)
            return;
        // Detect new heightmap data by checking blockHash change
        if (slot.blockHash !== currentHashRef.current && slot.heightmap) {
            currentHashRef.current = slot.blockHash;
            // Dispose old texture
            if (texRef.current)
                texRef.current.dispose();
            // Create new height texture
            const tex = createHeightTexture(slot.heightmap);
            texRef.current = tex;
            material.uniforms.uHeightmap.value = tex;
            material.uniforms.uActivityLevel.value = slot.activityLevel;
            material.uniforms.uHueShift.value = slot.params.hueShift;
            material.uniforms.uSaturation.value = slot.params.saturation;
        }
        meshRef.current.position.z = slot.positionZ;
        meshRef.current.visible = slot.active && slot.opacity > 0;
        material.uniforms.uTime.value += delta;
        material.uniforms.uOpacity.value = slot.opacity;
    });
    return _jsx("mesh", { ref: meshRef, geometry: geometry, material: material });
}
