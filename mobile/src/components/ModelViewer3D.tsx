'use dom';

/**
 * A spin-and-zoom viewer for a glTF binary (.glb), such as the 3D board
 * models the PCB skill exports. Runs as an Expo DOM component: inside a web
 * view on Android, as plain DOM on the desktop app. The model bytes come
 * from the native side in base64 chunks (`readChunk`), so the web view never
 * needs the server address or the auth token.
 */
import { useEffect, useRef, useState, type CSSProperties } from 'react';
import type { DOMProps } from 'expo/dom';
import {
  Box3,
  type BufferGeometry,
  Color,
  DirectionalLight,
  type Material,
  Mesh,
  MeshStandardMaterial,
  NeutralToneMapping,
  type Object3D,
  PerspectiveCamera,
  PMREMGenerator,
  Scene,
  SRGBColorSpace,
  Vector3,
  WebGLRenderer,
} from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js';
import { mergeGeometries } from 'three/examples/jsm/utils/BufferGeometryUtils.js';

type View = 'angle' | 'top' | 'bottom' | 'edge';

/**
 * Turns the loaded scene into a few large meshes, one per material.
 *
 * A KiCad board export holds thousands of small meshes (every pad, via and
 * part body), which means thousands of draw calls per frame and choppy
 * movement on a phone. Merging them by material brings that down to a
 * handful, so the model turns smoothly. KiCad also writes its colours as
 * sRGB where glTF expects linear values (pale solder mask), and leaves part
 * bodies with no material at all; both are fixed here.
 */
/** GLTFLoader's stand-in for a primitive with no material: unnamed, white, fully metallic and rough. */
function isLoaderDefault(m: Material): boolean {
  const s = m as MeshStandardMaterial;
  return !s.name && s.metalness === 1 && s.roughness === 1 && s.color?.getHex() === 0xffffff && !s.map;
}

function optimise(root: Object3D, fromKiCad: boolean, isDefault: (m: Material) => boolean): Object3D {
  const body = new MeshStandardMaterial({ color: 0x6b7078, roughness: 0.5, metalness: 0.1 });
  const groups = new Map<string, { material: Material; geoms: BufferGeometry[] }>();
  const fixed = new Set<Material>();
  root.updateMatrixWorld(true);
  root.traverse((o) => {
    const m = o as Mesh;
    if (!m.isMesh || Array.isArray(m.material)) return;
    const material = !m.material || isDefault(m.material) ? body : m.material;
    if (fromKiCad && material !== body && !fixed.has(material)) {
      const std = material as MeshStandardMaterial;
      std.color?.convertSRGBToLinear();
      // KiCad's see-through solder mask lets the gold copper tint the whole board olive;
      // a denser mask reads as the green a real board is, with traces still faintly visible.
      if (std.transparent && std.color && std.color.g > std.color.r && std.color.g > std.color.b)
        std.opacity = Math.max(std.opacity, 0.9);
      fixed.add(material);
    }
    const g = m.geometry.clone().applyMatrix4(m.matrixWorld);
    const key = `${material.uuid}|${Object.keys(g.attributes).sort().join(',')}|${g.index ? 'i' : 'n'}`;
    const entry = groups.get(key) ?? { material, geoms: [] };
    entry.geoms.push(g);
    groups.set(key, entry);
  });
  const out = new Scene();
  for (const { material, geoms } of groups.values()) {
    const merged = mergeGeometries(geoms, false);
    geoms.forEach((g) => g.dispose());
    if (merged) out.add(new Mesh(merged, material));
  }
  return out;
}

/** Camera direction for each preset: [azimuth, polar] in degrees, polar 0 = from above. */
const VIEWS: Record<View, [number, number]> = {
  angle: [-20, 55],
  top: [0, 0.01],
  bottom: [0, 179.99],
  edge: [0, 88],
};

const LABELS: [View, string][] = [
  ['angle', 'Angle'],
  ['top', 'Top'],
  ['bottom', 'Bottom'],
  ['edge', 'Edge'],
];

export default function ModelViewer3D({
  chunks,
  readChunk,
  background = '#141414',
  accent = '#8ab4f8',
  text = '#e3e3e3',
  onError,
}: {
  dom?: DOMProps;
  /** How many chunks `readChunk` serves. */
  chunks: number;
  /** Returns chunk `i` of the model, base64. */
  readChunk: (i: number) => Promise<string>;
  background?: string;
  accent?: string;
  text?: string;
  onError?: (message: string) => Promise<void> | void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const setView = useRef<(v: View) => void>(() => {});
  const [view, setViewState] = useState<View>('angle');
  const [status, setStatus] = useState('Loading the model…');

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    let disposed = false;

    const reduceMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
    const renderer = new WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' });
    // Sharp on phone screens without paying for 3x+ pixel ratios.
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    renderer.outputColorSpace = SRGBColorSpace;
    // Neutral tone mapping keeps KiCad's solder mask and silkscreen colours true.
    renderer.toneMapping = NeutralToneMapping;
    renderer.setClearColor(new Color(background));
    renderer.domElement.style.display = 'block';
    renderer.domElement.style.touchAction = 'none';
    el.appendChild(renderer.domElement);

    const scene = new Scene();
    const pmrem = new PMREMGenerator(renderer);
    scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    scene.environmentIntensity = 0.9;
    const sun = new DirectionalLight(0xffffff, 1.6);
    sun.position.set(-1, 3, 2);
    scene.add(sun);
    // A softer light from below, so the bottom side reads as clearly as the top.
    const fill = new DirectionalLight(0xffffff, 0.9);
    fill.position.set(1, -3, -1);
    scene.add(fill);

    const camera = new PerspectiveCamera(30, 1, 0.001, 1000);
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = !reduceMotion;
    controls.dampingFactor = 0.12;
    controls.rotateSpeed = 0.9;
    controls.zoomToCursor = true;

    const center = new Vector3();
    let radius = 1;

    // Render only when something moves: no frames, and no battery, spent on a still model.
    let frame = 0;
    let tween: ((now: number) => boolean) | null = null;
    const draw = (now: number) => {
      frame = 0;
      const tweening = tween ? tween(now) : false;
      if (!tweening) tween = null;
      const moving = controls.update();
      // Fit the depth range tightly around the model each frame. The solder mask sits
      // microns above the copper, and a loose range makes the two flicker through each other.
      const dist = camera.position.distanceTo(center);
      camera.near = Math.max(dist - radius * 1.1, radius * 0.02);
      camera.far = dist + radius * 1.1;
      camera.updateProjectionMatrix();
      renderer.render(scene, camera);
      if (tweening || moving) request();
    };
    const request = () => {
      if (!frame && !disposed) frame = requestAnimationFrame(draw);
    };
    controls.addEventListener('change', request);

    /** Camera position for a preset, far enough back to fit the whole model on screen. */
    const goal = (v: View) => {
      const [az, pol] = VIEWS[v];
      const a = (az * Math.PI) / 180;
      const p = (pol * Math.PI) / 180;
      const vHalf = (camera.fov / 2) * (Math.PI / 180);
      const hHalf = Math.atan(Math.tan(vHalf) * camera.aspect);
      const dist = (radius / Math.sin(Math.min(vHalf, hHalf))) * 1.02;
      return new Vector3(
        center.x + dist * Math.sin(p) * Math.sin(a),
        center.y + dist * Math.cos(p),
        center.z + dist * Math.sin(p) * Math.cos(a),
      );
    };
    const place = (v: View, animate: boolean) => {
      const to = goal(v);
      controls.target.copy(center);
      if (!animate || reduceMotion) {
        camera.position.copy(to);
        request();
        return;
      }
      // Glide along the orbit (around the centre), easing in and out.
      const from = camera.position.clone().sub(center);
      const end = to.clone().sub(center);
      const r0 = from.length();
      const r1 = end.length();
      const d0 = from.normalize();
      const d1 = end.normalize();
      const t0 = performance.now();
      const ms = 450;
      tween = (now) => {
        const t = Math.min(1, (now - t0) / ms);
        const k = t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
        const dir = d0.clone().lerp(d1, k);
        if (dir.lengthSq() < 1e-6) dir.copy(d1);
        camera.position.copy(center).addScaledVector(dir.normalize(), r0 + (r1 - r0) * k);
        return t < 1;
      };
      request();
    };
    setView.current = (v) => {
      setViewState(v);
      place(v, true);
    };

    const resize = () => {
      const w = el.clientWidth || 1;
      const h = el.clientHeight || 1;
      renderer.setSize(w, h, false);
      renderer.domElement.style.width = `${w}px`;
      renderer.domElement.style.height = `${h}px`;
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      request();
    };
    const ro = new ResizeObserver(resize);
    ro.observe(el);
    resize();

    (async () => {
      try {
        // Pull the model across in chunks and stitch the bytes together.
        const parts: Uint8Array[] = [];
        let total = 0;
        for (let i = 0; i < chunks; i++) {
          const b64 = await readChunk(i);
          const bin = atob(b64);
          const bytes = new Uint8Array(bin.length);
          for (let j = 0; j < bin.length; j++) bytes[j] = bin.charCodeAt(j);
          parts.push(bytes);
          // Let the page breathe between chunks so the progress text updates.
          await new Promise((r) => setTimeout(r, 0));
          total += bytes.length;
          if (disposed) return;
          setStatus(`Loading the model… ${Math.round(((i + 1) / chunks) * 100)}%`);
        }
        const buf = new Uint8Array(total);
        let off = 0;
        for (const p of parts) {
          buf.set(p, off);
          off += p.length;
        }
        parts.length = 0;
        const gltf = await new GLTFLoader().parseAsync(buf.buffer, '');
        if (disposed) return;
        const generator = String((gltf.asset as { extras?: { generator?: string } }).extras?.generator ?? '');
        const model = optimise(gltf.scene, generator.startsWith('KiCad'), isLoaderDefault);
        scene.add(model);
        const box = new Box3().setFromObject(model);
        box.getCenter(center);
        radius = Math.max(box.getSize(new Vector3()).length() / 2, 1e-4);
        controls.minDistance = radius * 0.2;
        controls.maxDistance = radius * 10;
        place('angle', false);
        setStatus('');
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setStatus(`Couldn't show this model: ${msg}`);
        void onError?.(msg);
      }
    })();

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      ro.disconnect();
      scene.traverse((o) => {
        const m = o as Mesh;
        if (!m.isMesh) return;
        m.geometry.dispose();
        (Array.isArray(m.material) ? m.material : [m.material]).forEach((x) => x.dispose());
      });
      controls.dispose();
      pmrem.dispose();
      renderer.dispose();
      renderer.domElement.remove();
    };
    // The model is loaded once per mount; the screen remounts for a new file.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const button = (active: boolean): CSSProperties => ({
    font: '500 14px system-ui, sans-serif',
    color: active ? accent : text,
    background: 'rgba(0,0,0,0.45)',
    border: `1px solid ${active ? accent : 'rgba(255,255,255,0.18)'}`,
    borderRadius: 8,
    padding: '8px 14px',
    cursor: 'pointer',
  });

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        background,
        overflow: 'hidden',
        userSelect: 'none',
        WebkitUserSelect: 'none',
      }}
    >
      <div ref={host} style={{ position: 'absolute', inset: 0 }} />
      {status ? (
        <div
          role="status"
          style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            padding: 24,
            textAlign: 'center',
            color: text,
            font: '15px system-ui, sans-serif',
            pointerEvents: 'none',
          }}
        >
          {status}
        </div>
      ) : (
        <div
          role="group"
          aria-label="Camera"
          style={{
            position: 'absolute',
            left: 12,
            right: 12,
            bottom: 16,
            display: 'flex',
            flexWrap: 'wrap',
            gap: 8,
            justifyContent: 'center',
          }}
        >
          {LABELS.map(([v, label]) => (
            <button
              key={v}
              type="button"
              aria-pressed={view === v}
              style={button(view === v)}
              onClick={() => setView.current(v)}
            >
              {label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
