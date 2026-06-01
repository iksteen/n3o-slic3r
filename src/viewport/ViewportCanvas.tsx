// 3D viewport (PR-2-9).
//
// React shell that owns the Three.js renderer + scene graph and
// drives the SceneMirror / event bridge lifecycle. The scene graph
// itself is built inside `useEffect` so we have a single Three.js
// world per component mount (StrictMode's double-mount path is
// handled by the cleanup function).
//
// Most user intent flows back through Tauri commands rather than
// being stored locally: clicking an object → `scene_select`, deleting
// a selection → `scene_object_delete`, etc. The renderer is a
// reflector; the canonical state is on the Rust side.

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type ReactNode } from "react";
import * as THREE from "three";
import type { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  initialFrameForBed,
  makeControls,
  makePerspectiveCamera,
} from "./cameraControls";
import { attachEventBridge, tauriMeshBufferProvider } from "./eventBridge";
import { createGizmo, type GizmoApi } from "./gizmo";
import { SceneMirror } from "./sceneMirror";
import type { GizmoMode, ObjectId } from "./types";

interface ToastMessage {
  id: number;
  level: "info" | "warn" | "error";
  text: string;
}

let nextToastId = 1;

// Transform-gizmo mode icons, ported from the design mockup
// (docs/design/app.jsx vp-toolbar): move arrows, a rotate arc, and a
// scale corner-handle pair.
const GIZMO_ICONS: Record<GizmoMode, ReactNode> = {
  Translate: (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M12 2v20M2 12h20M12 2l-3 3M12 2l3 3M12 22l-3-3M12 22l3-3M2 12l3-3M2 12l3 3M22 12l-3-3M22 12l-3 3" />
    </svg>
  ),
  Rotate: (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M21 12a9 9 0 1 1-3-6.7" />
      <path d="M21 3v5h-5" />
    </svg>
  ),
  Scale: (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M3 3h7M3 3v7M3 3l7 7M21 21h-7M21 21v-7M21 21l-7-7" />
    </svg>
  ),
};

export function ViewportCanvas({
  leading,
  gizmoMode,
  onGizmoMode,
}: {
  leading?: ReactNode;
  /** Active transform mode. Owned by App (not backend scene state, not
   *  this component) so it survives the unmount/remount this component
   *  goes through on prepare↔preview↔devices mode switches. */
  gizmoMode: GizmoMode;
  onGizmoMode: (mode: GizmoMode) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const mirrorRef = useRef<SceneMirror | null>(null);
  // Live gizmo handle, so the always-visible toolbar buttons can drive
  // the gizmo's transform mode directly (mode is renderer-local — held
  // in App, not round-tripped through backend scene state).
  const gizmoRef = useRef<GizmoApi | null>(null);
  const [toasts, setToasts] = useState<ToastMessage[]>([]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    // ---- Renderer + scene boot ---------------------------------------
    const renderer = new THREE.WebGLRenderer({
      antialias: true,
      preserveDrawingBuffer: false,
    });
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setClearColor(0x1a1a1a);
    container.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x1a1a1a);

    // Lighting: ambient + a key light from above-front so faces have
    // dimension without harsh shadows. Three-point is overkill for a
    // build-plate view.
    scene.add(new THREE.AmbientLight(0xffffff, 0.55));
    const key = new THREE.DirectionalLight(0xffffff, 0.9);
    key.position.set(150, -150, 250);
    scene.add(key);
    const fill = new THREE.DirectionalLight(0xffffff, 0.25);
    fill.position.set(-150, 150, 100);
    scene.add(fill);

    let aspect = container.clientWidth / Math.max(container.clientHeight, 1);
    const camera = makePerspectiveCamera(aspect);
    const controls: OrbitControls = makeControls(camera, renderer.domElement);

    // Mirror + groups.
    const mirror = new SceneMirror(tauriMeshBufferProvider());
    mirrorRef.current = mirror;
    scene.add(mirror.objectGroup);
    scene.add(mirror.bedGroup);

    // Gizmo. TransformControls subscribes to `dragging-changed` so
    // OrbitControls can be paused for the duration of a drag —
    // otherwise the orbit fights the drag.
    const gizmo: GizmoApi = createGizmo({
      camera,
      domElement: renderer.domElement,
      scene,
      mirror,
    });
    gizmo.controls.addEventListener("dragging-changed", (ev) => {
      const dragging = (ev as { value: boolean }).value;
      controls.enabled = !dragging;
    });
    gizmoRef.current = gizmo;

    // ---- Event bridge -------------------------------------------------
    //
    // PR-5-2 phase C: events route to the active plate via SceneMirror.
    // The viewport only reacts to events whose `plate_id` matches the
    // active plate — toasts / gizmo / camera updates for an inactive
    // plate would render against the wrong workspace. (Frontend
    // PlateTabs receives ActivePlateChanged separately and swaps the
    // viewport's framing.)
    const detachToastsListener = mirror.onEvent((evt) => {
      const activeId = mirror.activePlateIdOrNull();
      switch (evt.kind) {
        case "ObjectOutOfBounds":
          if (evt.data.plate_id !== activeId) break;
          pushToast(
            "warn",
            `object ${evt.data.object_id} out of bounds: ${evt.data.reasons
              .map((r) => r.kind)
              .join(", ")}`,
          );
          break;
        case "NonUniformScale":
          if (evt.data.plate_id !== activeId) break;
          pushToast(
            "warn",
            `object ${evt.data.object_id} now has non-uniform scale — dimensional settings may be off`,
          );
          break;
        case "AutoArrangeOverflow":
          if (evt.data.plate_id !== activeId) break;
          pushToast(
            "warn",
            `auto-arrange could not place ${evt.data.un_placed.length} object(s)`,
          );
          break;
        case "SelectionChanged":
          if (evt.data.plate_id !== activeId) break;
          gizmo.setSelection(evt.data.selected);
          break;
        case "BedChanged":
          if (evt.data.plate_id !== activeId) break;
          if (evt.data.bed) {
            initialFrameForBed(camera, controls, evt.data.bed);
          }
          break;
        case "ActivePlateChanged": {
          // The active plate just changed — re-sync the viewport's
          // selection + camera framing from the new plate's cached
          // state so the workspace matches. (Transform mode is
          // App-owned and survives the switch.)
          const plate = mirror.activePlate();
          if (plate) {
            gizmo.setSelection(
              Array.from(plate.selection).sort((a, b) => a - b),
            );
            if (plate.bed) {
              initialFrameForBed(camera, controls, plate.bed);
            }
          }
          break;
        }
      }
    });

    let detachBridge: (() => Promise<void>) | null = null;
    attachEventBridge(mirror)
      .then((un) => {
        // The bed comes from the snapshot the bridge applies on attach
        // (Project::default() binds the first library instance). An
        // unbound plate (empty library) has no bed — the onboarding
        // empty-state covers that case, no default printer is forced.
        detachBridge = un;
      })
      .catch((err) => {
        pushToast("error", `viewport init failed: ${err}`);
      });

    // ---- Resize handling ---------------------------------------------
    const onResize = () => {
      const w = container.clientWidth;
      const h = Math.max(container.clientHeight, 1);
      aspect = w / h;
      renderer.setSize(w, h, false);
      camera.aspect = aspect;
      camera.updateProjectionMatrix();
    };
    onResize();
    const resizeObserver = new ResizeObserver(onResize);
    resizeObserver.observe(container);

    // ---- Click-to-select ---------------------------------------------
    const raycaster = new THREE.Raycaster();
    const pointer = new THREE.Vector2();
    const onClick = (ev: MouseEvent) => {
      if (ev.target !== renderer.domElement) return;
      const rect = renderer.domElement.getBoundingClientRect();
      pointer.x = ((ev.clientX - rect.left) / rect.width) * 2 - 1;
      pointer.y = -((ev.clientY - rect.top) / rect.height) * 2 + 1;
      raycaster.setFromCamera(pointer, camera);
      // Recursive: post-PR-5-2 phase C, `mirror.objectGroup`'s
      // direct children are the per-plate wrapper groups, not the
      // meshes themselves. The meshes live one level down on
      // `activePlate.objectGroup`. A non-recursive walk finds the
      // wrapper group (no geometry) and reports zero hits, which
      // is why object selection silently stopped working once the
      // per-plate restructure landed.
      const hits = raycaster.intersectObjects(
        mirror.objectGroup.children,
        true,
      );
      const additive = ev.shiftKey || ev.metaKey || ev.ctrlKey;
      if (hits.length === 0) {
        if (!additive) {
          void invoke("scene_deselect");
        }
        return;
      }
      const id = (hits[0].object.userData.objectId as ObjectId) ?? null;
      if (id !== null) {
        void invoke("scene_select", {
          ids: [id],
          mode: additive ? "Add" : "Replace",
        });
      }
    };
    renderer.domElement.addEventListener("click", onClick);

    // ---- Keyboard shortcuts ------------------------------------------
    const onKeyDown = (ev: KeyboardEvent) => {
      if ((ev.key === "Delete" || ev.key === "Backspace") && selectedSnapshot().length > 0) {
        void invoke("scene_object_delete", { ids: selectedSnapshot() });
      }
    };
    const selectedSnapshot = () => mirror.selectedIds();
    window.addEventListener("keydown", onKeyDown);

    // ---- Animation loop ----------------------------------------------
    let raf = 0;
    const render = () => {
      controls.update();
      renderer.render(scene, camera);
      raf = requestAnimationFrame(render);
    };
    raf = requestAnimationFrame(render);

    return () => {
      cancelAnimationFrame(raf);
      renderer.domElement.removeEventListener("click", onClick);
      window.removeEventListener("keydown", onKeyDown);
      resizeObserver.disconnect();
      detachToastsListener();
      if (detachBridge) void detachBridge();
      gizmoRef.current = null;
      gizmo.dispose();
      controls.dispose();
      mirror.clear();
      renderer.dispose();
      if (renderer.domElement.parentElement === container) {
        container.removeChild(renderer.domElement);
      }
    };

    function pushToast(level: ToastMessage["level"], text: string) {
      const id = nextToastId++;
      setToasts((prev) => [...prev, { id, level, text }]);
      setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, 4000);
    }
  }, []);

  // Apply the active transform mode to the live gizmo — on mount (after
  // the setup effect created the gizmo and set gizmoRef), on every
  // toolbar change, and after a remount with an App-preserved mode.
  useEffect(() => {
    gizmoRef.current?.setMode(gizmoMode);
  }, [gizmoMode]);

  return (
    <div className="relative w-full h-full">
      <div ref={containerRef} className="w-full h-full" />
      <div className="absolute top-2 left-2 flex flex-col gap-1 pointer-events-none">
        <div className="flex gap-2 pointer-events-auto">
          {leading}
          <div className="bg-neutral-800/90 text-neutral-100 text-xs rounded shadow flex overflow-hidden">
            {(["Translate", "Rotate", "Scale"] as GizmoMode[]).map((mode) => (
              <button
                key={mode}
                type="button"
                className={`px-2 py-1.5 ${
                  gizmoMode === mode
                    ? "bg-neutral-700"
                    : "hover:bg-neutral-700/60"
                }`}
                onClick={() => onGizmoMode(mode)}
                title={`Gizmo: ${mode}`}
                aria-label={mode}
                aria-pressed={gizmoMode === mode}
              >
                {GIZMO_ICONS[mode]}
              </button>
            ))}
          </div>
        </div>
      </div>
      <div className="gizmo-hint pointer-events-none">
        <span className="axes" aria-label="Axes">
          <span className="axis axis-x">X</span>
          <span className="axis axis-y">Y</span>
          <span className="axis axis-z">Z</span>
        </span>
        <span className="gizmo-hint-sep" aria-hidden>
          ·
        </span>
        Drag · LMB rotate · RMB pan · scroll zoom
      </div>
      <div className="absolute bottom-2 right-2 flex flex-col gap-1 pointer-events-none">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={`pointer-events-auto text-xs px-3 py-2 rounded shadow text-neutral-100 ${
              t.level === "error"
                ? "bg-red-700/90"
                : t.level === "warn"
                ? "bg-amber-700/90"
                : "bg-neutral-700/90"
            }`}
          >
            {t.text}
          </div>
        ))}
      </div>
    </div>
  );
}

