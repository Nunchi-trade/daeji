import { jsx as _jsx } from "react/jsx-runtime";
import { useRef, useMemo, useCallback } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
/** Max beams to render at once (performance budget). */
const MAX_BEAMS = 64;
const BEAM_FADE_DURATION = 10; // seconds
const BEAM_WIDTH = 0.05;
/**
 * Derive a beam X offset within the tile from a transaction hash.
 * Uses first 8 hex chars for X, next 8 for Z.
 */
function txToBeamOffset(txHash) {
    const x = parseInt(txHash.slice(2, 10), 16);
    const z = parseInt(txHash.slice(10, 18), 16);
    const nx = ((x % 1000) / 1000 - 0.5) * 6;
    const nz = ((z % 1000) / 1000 - 0.5) * 6;
    return [nx, nz];
}
/** Beam height from gas usage relative to block gas limit. */
function txToBeamHeight(tx, gasLimit) {
    if (gasLimit === 0n)
        return 0.5;
    const ratio = Number(tx.gas) / Number(gasLimit);
    return 0.3 + ratio * 2.0;
}
/**
 * Renders additive-blended vertical beam quads for transactions
 * on visible terrain tiles.
 */
export function TransactionBeams({ ring }) {
    const groupRef = useRef(null);
    const beamPool = useRef([]);
    const beamIndex = useRef(0);
    const beamMaterial = useMemo(() => new THREE.MeshBasicMaterial({
        color: 0xd8c8a0, // bone-bright
        transparent: true,
        opacity: 0.6,
        blending: THREE.AdditiveBlending,
        side: THREE.DoubleSide,
        depthWrite: false,
    }), []);
    const beamGeometry = useMemo(() => new THREE.PlaneGeometry(BEAM_WIDTH, 1, 1, 1), []);
    // Lazy pool creation
    const getBeam = useCallback(() => {
        if (!groupRef.current)
            return null;
        if (beamIndex.current < beamPool.current.length) {
            const mesh = beamPool.current[beamIndex.current];
            mesh.visible = true;
            beamIndex.current++;
            return mesh;
        }
        if (beamPool.current.length >= MAX_BEAMS) {
            // Recycle oldest
            const mesh = beamPool.current[beamIndex.current % MAX_BEAMS];
            beamIndex.current = (beamIndex.current + 1) % MAX_BEAMS;
            mesh.visible = true;
            return mesh;
        }
        const mesh = new THREE.Mesh(beamGeometry, beamMaterial.clone());
        mesh.visible = false;
        groupRef.current.add(mesh);
        beamPool.current.push(mesh);
        beamIndex.current = beamPool.current.length;
        return mesh;
    }, [beamGeometry, beamMaterial]);
    useFrame(() => {
        if (!groupRef.current)
            return;
        const now = performance.now();
        // Spawn beams for active slots with transactions
        for (const slot of ring.getSlots()) {
            if (!slot.active || slot.transactions.length === 0)
                continue;
            for (const tx of slot.transactions) {
                // Check if beam already exists for this tx
                const existing = beamPool.current.find((m) => m.userData.txHash === tx.hash);
                if (existing)
                    continue;
                const beam = getBeam();
                if (!beam)
                    continue;
                const [offsetX, offsetZ] = txToBeamOffset(tx.hash);
                // Default gasLimit for height calculation
                const gasLimit = 30000000n;
                const height = txToBeamHeight(tx, gasLimit);
                beam.scale.set(1, height, 1);
                beam.position.set(offsetX, height / 2, slot.positionZ + offsetZ);
                beam.userData.txHash = tx.hash;
                beam.userData.birthTime = now;
                beam.userData.tileId = slot.id;
                // Color: bone-bright for value transfer, rose for contract creation
                const mat = beam.material;
                if (tx.to === null) {
                    mat.color.setHex(0xcc90a8); // rose-bright (contract creation)
                }
                else {
                    mat.color.setHex(0xd8c8a0); // bone-bright (value transfer)
                }
                mat.opacity = 0.6;
            }
        }
        // Update beam fade + position tracking
        for (const mesh of beamPool.current) {
            if (!mesh.visible)
                continue;
            const birthTime = mesh.userData.birthTime;
            const age = (now - birthTime) / 1000;
            const fadeProgress = Math.min(age / BEAM_FADE_DURATION, 1);
            mesh.material.opacity =
                0.6 * (1 - fadeProgress);
            if (fadeProgress >= 1) {
                mesh.visible = false;
                mesh.userData.txHash = null;
            }
            // Track tile position so beams scroll with terrain
            const tileId = mesh.userData.tileId;
            const slot = ring.getSlots().find((s) => s.id === tileId);
            if (slot && slot.active) {
                const [, offsetZ] = mesh.userData.txHash
                    ? txToBeamOffset(mesh.userData.txHash)
                    : [0, 0];
                mesh.position.z = slot.positionZ + offsetZ;
            }
        }
    });
    return _jsx("group", { ref: groupRef });
}
