import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useRef, useEffect, useCallback } from 'react';
import createREGL from 'regl';
import { bus } from '@/data/bus';
import { clamp } from '@/lib/math';
import { AddressBook } from './nodes';
import { ArcManager, bezierPoint } from './arcs';
import { ParticleSystem, PARTICLE_FLOATS_PER } from './particles';
import fragShader from './constellation.frag.glsl';
// ── Vertex shader (inline, small) ───────────────────────────
const VERT_SHADER = `
  precision highp float;
  attribute vec2 aPosition;
  attribute vec4 aColor;
  attribute float aSize;
  uniform vec2 uPan;
  uniform float uZoom;
  uniform vec2 uResolution;
  varying vec4 vColor;
  void main() {
    vColor = aColor;
    vec2 pos = (aPosition + uPan) * uZoom;
    // Aspect-correct: map to clip space
    pos.x *= uResolution.y / uResolution.x;
    gl_Position = vec4(pos, 0.0, 1.0);
    gl_PointSize = aSize * uZoom;
  }
`;
// ── Ghost-line vertex/fragment (for persisting arc paths) ────
const LINE_VERT = `
  precision highp float;
  attribute vec2 aPosition;
  uniform vec2 uPan;
  uniform float uZoom;
  uniform vec2 uResolution;
  void main() {
    vec2 pos = (aPosition + uPan) * uZoom;
    pos.x *= uResolution.y / uResolution.x;
    gl_Position = vec4(pos, 0.0, 1.0);
  }
`;
const LINE_FRAG = `
  precision highp float;
  uniform vec4 uColor;
  void main() {
    gl_FragColor = uColor;
  }
`;
function createCamera() {
    return {
        panX: 0,
        panY: 0,
        zoom: 1.5,
        targetPanX: 0,
        targetPanY: 0,
        targetZoom: 1.5,
        dragging: false,
        dragStartX: 0,
        dragStartY: 0,
        dragPanStartX: 0,
        dragPanStartY: 0,
    };
}
// ── Orbital drift ────────────────────────────────────────────
const DRIFT_SPEED = 0.001; // rad/s
// ── Component ────────────────────────────────────────────────
function ConstellationCanvas() {
    const canvasRef = useRef(null);
    const cleanupRef = useRef(null);
    const init = useCallback((canvas) => {
        // ── WebGL2 check ──
        const gl = canvas.getContext('webgl2', {
            antialias: true,
            alpha: true,
            premultipliedAlpha: false,
            powerPreference: 'high-performance',
        });
        if (!gl)
            return null;
        // ── Init regl on this context ──
        const regl = createREGL({ gl, extensions: ['OES_element_index_uint'] });
        // ── Core systems ──
        const addressBook = new AddressBook(4096);
        const arcManager = new ArcManager(256);
        const particles = new ParticleSystem(16384);
        const camera = createCamera();
        // Preallocate GPU buffers
        const nodePositionBuf = regl.buffer({ usage: 'dynamic', type: 'float', length: 4096 * 2 * 4 });
        const nodeColorBuf = regl.buffer({ usage: 'dynamic', type: 'float', length: 4096 * 4 * 4 });
        const nodeSizeBuf = regl.buffer({ usage: 'dynamic', type: 'float', length: 4096 * 1 * 4 });
        const particlePositionBuf = regl.buffer({ usage: 'dynamic', type: 'float', length: 16384 * 2 * 4 });
        const particleColorBuf = regl.buffer({ usage: 'dynamic', type: 'float', length: 16384 * 4 * 4 });
        const particleSizeBuf = regl.buffer({ usage: 'dynamic', type: 'float', length: 16384 * 1 * 4 });
        const linePositionBuf = regl.buffer({ usage: 'dynamic', type: 'float', length: 256 * 32 * 2 * 4 });
        // Temp CPU arrays for batching
        const nodePos = new Float32Array(4096 * 2);
        const nodeCol = new Float32Array(4096 * 4);
        const nodeSize = new Float32Array(4096);
        const particlePos = new Float32Array(16384 * 2);
        const particleCol = new Float32Array(16384 * 4);
        const particleSz = new Float32Array(16384);
        const lineVerts = new Float32Array(256 * 32 * 2);
        // ── regl draw commands ──
        const drawPoints = regl({
            vert: VERT_SHADER,
            frag: fragShader,
            attributes: {
                aPosition: { buffer: () => nodePositionBuf, size: 2 },
                aColor: { buffer: () => nodeColorBuf, size: 4 },
                aSize: { buffer: () => nodeSizeBuf, size: 1 },
            },
            uniforms: {
                uPan: () => [camera.panX, camera.panY],
                uZoom: () => camera.zoom,
                uResolution: (ctx) => [
                    ctx.viewportWidth,
                    ctx.viewportHeight,
                ],
            },
            count: () => addressBook.size,
            primitive: 'points',
            blend: {
                enable: true,
                func: { srcRGB: 'src alpha', dstRGB: 'one minus src alpha', srcAlpha: 'one', dstAlpha: 'one minus src alpha' },
            },
            depth: { enable: false },
        });
        const drawParticles = regl({
            vert: VERT_SHADER,
            frag: fragShader,
            attributes: {
                aPosition: { buffer: () => particlePositionBuf, size: 2 },
                aColor: { buffer: () => particleColorBuf, size: 4 },
                aSize: { buffer: () => particleSizeBuf, size: 1 },
            },
            uniforms: {
                uPan: () => [camera.panX, camera.panY],
                uZoom: () => camera.zoom,
                uResolution: (ctx) => [
                    ctx.viewportWidth,
                    ctx.viewportHeight,
                ],
            },
            count: () => particles.count,
            primitive: 'points',
            blend: {
                enable: true,
                func: { srcRGB: 'src alpha', dstRGB: 'one', srcAlpha: 'one', dstAlpha: 'one' },
            },
            depth: { enable: false },
        });
        const drawLines = regl({
            vert: LINE_VERT,
            frag: LINE_FRAG,
            attributes: {
                aPosition: { buffer: () => linePositionBuf, size: 2 },
            },
            uniforms: {
                uPan: () => [camera.panX, camera.panY],
                uZoom: () => camera.zoom,
                uResolution: (ctx) => [
                    ctx.viewportWidth,
                    ctx.viewportHeight,
                ],
                uColor: regl.prop('uColor'),
            },
            count: regl.prop('count'),
            primitive: 'line strip',
            blend: {
                enable: true,
                func: { srcRGB: 'src alpha', dstRGB: 'one minus src alpha', srcAlpha: 'one', dstAlpha: 'one minus src alpha' },
            },
            depth: { enable: false },
            lineWidth: 1,
        });
        // ── EventBus subscriptions ──
        const onBlock = (block) => {
            for (const tx of block.transactions) {
                const fromNode = addressBook.getOrCreate(tx.from);
                addressBook.markActive(fromNode);
                if (tx.to) {
                    const toNode = addressBook.getOrCreate(tx.to);
                    addressBook.markActive(toNode);
                    if (tx.input.length > 2) {
                        addressBook.markContract(toNode);
                    }
                }
            }
        };
        const onTxConfirmed = (ev) => {
            const receipt = ev.receipt;
            const fromNode = addressBook.getOrCreate(receipt.from);
            addressBook.markActive(fromNode);
            if (receipt.to) {
                const toNode = addressBook.getOrCreate(receipt.to);
                addressBook.markActive(toNode);
                const arc = arcManager.addArc(fromNode, toNode, 0n, ev.hash, 'confirmed');
                const count = 8 + Math.floor(Math.random() * 9);
                particles.spawnArcParticles(arc, count);
            }
            if (receipt.contractAddress) {
                const contractNode = addressBook.getOrCreate(receipt.contractAddress);
                addressBook.markContract(contractNode);
            }
        };
        const onTxPending = (tx) => {
            const fromNode = addressBook.getOrCreate(tx.from);
            if (tx.to) {
                const toNode = addressBook.getOrCreate(tx.to);
                arcManager.addArc(fromNode, toNode, tx.value, tx.hash, 'pending');
            }
        };
        const onAddressSeen = (addr) => {
            addressBook.getOrCreate(addr);
        };
        bus.on('block:new', onBlock);
        bus.on('tx:confirmed', onTxConfirmed);
        bus.on('tx:pending', onTxPending);
        bus.on('address:seen', onAddressSeen);
        // ── Mouse handlers ──
        const onWheel = (e) => {
            e.preventDefault();
            const factor = e.deltaY > 0 ? 0.9 : 1.1;
            camera.targetZoom = clamp(camera.targetZoom * factor, 0.3, 20);
        };
        const onMouseDown = (e) => {
            if (e.button !== 0)
                return;
            camera.dragging = true;
            camera.dragStartX = e.clientX;
            camera.dragStartY = e.clientY;
            camera.dragPanStartX = camera.targetPanX;
            camera.dragPanStartY = camera.targetPanY;
        };
        const onMouseMove = (e) => {
            if (!camera.dragging)
                return;
            const dx = (e.clientX - camera.dragStartX) / (canvas.clientWidth * camera.zoom * 0.5);
            const dy = -(e.clientY - camera.dragStartY) / (canvas.clientHeight * camera.zoom * 0.5);
            camera.targetPanX = camera.dragPanStartX + dx;
            camera.targetPanY = camera.dragPanStartY + dy;
        };
        const onMouseUp = () => {
            camera.dragging = false;
        };
        const onDblClick = (e) => {
            // Convert screen coords to world coords, zoom in
            const rect = canvas.getBoundingClientRect();
            const nx = ((e.clientX - rect.left) / rect.width) * 2 - 1;
            const ny = -(((e.clientY - rect.top) / rect.height) * 2 - 1);
            const aspect = rect.width / rect.height;
            camera.targetPanX = -nx * aspect / camera.zoom;
            camera.targetPanY = -ny / camera.zoom;
            camera.targetZoom = clamp(camera.targetZoom * 2, 0.3, 20);
        };
        canvas.addEventListener('wheel', onWheel, { passive: false });
        canvas.addEventListener('mousedown', onMouseDown);
        window.addEventListener('mousemove', onMouseMove);
        window.addEventListener('mouseup', onMouseUp);
        canvas.addEventListener('dblclick', onDblClick);
        // ── Resize handling ──
        const resizeCanvas = () => {
            const dpr = Math.min(window.devicePixelRatio, 2);
            const w = canvas.clientWidth;
            const h = canvas.clientHeight;
            if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
                canvas.width = w * dpr;
                canvas.height = h * dpr;
            }
        };
        const resizeObserver = new ResizeObserver(resizeCanvas);
        resizeObserver.observe(canvas);
        resizeCanvas();
        // ── Animation loop ──
        let lastTime = performance.now();
        let driftAngle = 0;
        let fireflyTimer = 0;
        let animFrame = 0;
        const frame = () => {
            animFrame = requestAnimationFrame(frame);
            const now = performance.now();
            const dt = Math.min((now - lastTime) / 1000, 0.1); // cap delta
            lastTime = now;
            resizeCanvas();
            // ── Smooth camera ──
            camera.panX += (camera.targetPanX - camera.panX) * 0.08;
            camera.panY += (camera.targetPanY - camera.panY) * 0.08;
            camera.zoom += (camera.targetZoom - camera.zoom) * 0.08;
            // ── Orbital drift (idle) ──
            driftAngle += DRIFT_SPEED * dt;
            // ── Update systems ──
            addressBook.update(dt);
            arcManager.update(dt);
            particles.update(dt);
            // ── Firefly spawning (idle ambient) ──
            fireflyTimer += dt;
            if (fireflyTimer > 0.5) {
                fireflyTimer = 0;
                particles.spawnFireflies(0, 0, 1.0, 2);
            }
            // ── Upload node data to GPU buffers ──
            const nodes = addressBook.getNodeList();
            const nodeCount = nodes.length;
            const time = now / 1000;
            for (let i = 0; i < nodeCount; i++) {
                const n = nodes[i];
                const pulse = addressBook.pulseScale(n);
                const breath = addressBook.breathScale(n, time);
                const scale = pulse * breath;
                // Apply orbital drift rotation
                const cos = Math.cos(driftAngle);
                const sin = Math.sin(driftAngle);
                const rx = n.x * cos - n.y * sin;
                const ry = n.x * sin + n.y * cos;
                nodePos[i * 2] = rx;
                nodePos[i * 2 + 1] = ry;
                nodeCol[i * 4] = n.r;
                nodeCol[i * 4 + 1] = n.g;
                nodeCol[i * 4 + 2] = n.b;
                nodeCol[i * 4 + 3] = n.a;
                nodeSize[i] = n.size * scale;
            }
            if (nodeCount > 0) {
                nodePositionBuf.subdata(nodePos.subarray(0, nodeCount * 2));
                nodeColorBuf.subdata(nodeCol.subarray(0, nodeCount * 4));
                nodeSizeBuf.subdata(nodeSize.subarray(0, nodeCount));
            }
            // ── Upload particle data ──
            const pCount = particles.count;
            for (let i = 0; i < pCount; i++) {
                const base = i * PARTICLE_FLOATS_PER;
                particlePos[i * 2] = particles.data[base]; // x
                particlePos[i * 2 + 1] = particles.data[base + 1]; // y
                particleCol[i * 4] = particles.data[base + 6]; // r
                particleCol[i * 4 + 1] = particles.data[base + 7]; // g
                particleCol[i * 4 + 2] = particles.data[base + 8]; // b
                particleCol[i * 4 + 3] = particles.data[base + 9]; // a
                particleSz[i] = particles.data[base + 10]; // size
            }
            if (pCount > 0) {
                particlePositionBuf.subdata(particlePos.subarray(0, pCount * 2));
                particleColorBuf.subdata(particleCol.subarray(0, pCount * 4));
                particleSizeBuf.subdata(particleSz.subarray(0, pCount));
            }
            // ── Draw ──
            regl.clear({ color: [0.024, 0.024, 0.031, 1], depth: 1 }); // void #060608
            // Draw ghost arc lines
            const arcs = arcManager.getArcs();
            for (let a = 0; a < arcs.length; a++) {
                const arc = arcs[a];
                if (arc.opacity <= 0.01)
                    continue;
                const segs = arc.segments;
                for (let s = 0; s <= segs; s++) {
                    const t = s / segs;
                    const pt = bezierPoint(arc, t);
                    // Apply drift rotation
                    const cos = Math.cos(driftAngle);
                    const sin = Math.sin(driftAngle);
                    lineVerts[s * 2] = pt.x * cos - pt.y * sin;
                    lineVerts[s * 2 + 1] = pt.x * sin + pt.y * cos;
                }
                linePositionBuf.subdata(lineVerts.subarray(0, (segs + 1) * 2));
                const ghostAlpha = arc.progress >= 1 ? arc.opacity * 0.15 : arc.opacity * 0.4;
                drawLines({
                    count: segs + 1,
                    uColor: [0.847, 0.784, 0.627, ghostAlpha],
                });
            }
            // Draw nodes
            if (nodeCount > 0) {
                drawPoints();
            }
            // Draw particles (additive blend)
            if (pCount > 0) {
                drawParticles();
            }
        };
        animFrame = requestAnimationFrame(frame);
        // ── Cleanup ──
        return () => {
            cancelAnimationFrame(animFrame);
            bus.off('block:new', onBlock);
            bus.off('tx:confirmed', onTxConfirmed);
            bus.off('tx:pending', onTxPending);
            bus.off('address:seen', onAddressSeen);
            canvas.removeEventListener('wheel', onWheel);
            canvas.removeEventListener('mousedown', onMouseDown);
            window.removeEventListener('mousemove', onMouseMove);
            window.removeEventListener('mouseup', onMouseUp);
            canvas.removeEventListener('dblclick', onDblClick);
            resizeObserver.disconnect();
            regl.destroy();
        };
    }, []);
    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas)
            return;
        const cleanup = init(canvas);
        cleanupRef.current = cleanup;
        return () => {
            if (cleanupRef.current) {
                cleanupRef.current();
                cleanupRef.current = null;
            }
        };
    }, [init]);
    return (_jsxs("div", { style: { position: 'absolute', inset: 0, overflow: 'hidden', background: '#060608' }, children: [_jsx("canvas", { ref: canvasRef, style: { width: '100%', height: '100%', display: 'block' } }), _jsx("noscript", { children: _jsx("div", { style: { color: '#6a5a68', textAlign: 'center', paddingTop: '40vh' }, children: "WebGL2 is required for the constellation view." }) })] }));
}
export default ConstellationCanvas;
