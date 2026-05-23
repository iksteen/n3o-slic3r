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
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import type { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  frameBox,
  initialFrameForBed,
  makeControls,
  makeOrthographicCamera,
  makePerspectiveCamera,
} from "./cameraControls";
import { attachEventBridge, tauriMeshBufferProvider } from "./eventBridge";
import { createGizmo, type GizmoApi } from "./gizmo";
import { SceneMirror } from "./sceneMirror";
import type { GizmoMode, ObjectId, ProjectionMode } from "./types";

interface ToastMessage {
  id: number;
  level: "info" | "warn" | "error";
  text: string;
}

let nextToastId = 1;

export function ViewportCanvas() {
  const containerRef = useRef<HTMLDivElement>(null);
  const mirrorRef = useRef<SceneMirror | null>(null);
  const [projection, setProjection] = useState<ProjectionMode>("Perspective");
  const [toasts, setToasts] = useState<ToastMessage[]>([]);
  const [selectedIds, setSelectedIds] = useState<ObjectId[]>([]);
  const [gizmoMode, setGizmoMode] = useState<GizmoMode>("None");

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
    let camera: THREE.PerspectiveCamera | THREE.OrthographicCamera =
      makePerspectiveCamera(aspect);
    let controls: OrbitControls = makeControls(camera, renderer.domElement);

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

    // ---- Event bridge -------------------------------------------------
    const detachToastsListener = mirror.onEvent((evt) => {
      switch (evt.kind) {
        case "ObjectOutOfBounds":
          pushToast(
            "warn",
            `object ${evt.data.id} out of bounds: ${evt.data.reasons
              .map((r) => r.kind)
              .join(", ")}`,
          );
          break;
        case "NonUniformScale":
          pushToast(
            "warn",
            `object ${evt.data.id} now has non-uniform scale — dimensional settings may be off`,
          );
          break;
        case "AutoArrangeOverflow":
          pushToast(
            "warn",
            `auto-arrange could not place ${evt.data.un_placed.length} object(s)`,
          );
          break;
        case "SelectionChanged":
          setSelectedIds([...evt.data.selected]);
          gizmo.setSelection(evt.data.selected);
          break;
        case "GizmoChanged":
          setGizmoMode(evt.data.mode);
          gizmo.setMode(evt.data.mode);
          gizmo.setPivotOverride(evt.data.pivot);
          break;
        case "BedChanged":
          if (evt.data) {
            initialFrameForBed(camera, controls, evt.data, aspect);
          }
          break;
      }
    });

    let detachBridge: (() => Promise<void>) | null = null;
    attachEventBridge(mirror)
      .then(async (un) => {
        detachBridge = un;
        // Phase 2 bootstrap: pull the bundled A1 mini profile so the
        // viewport has a bed to render before Phase 5 wires real
        // printer selection. No-op if a printer is already active.
        if (!mirror.bed) {
          try {
            await invoke("scene_load_default_printer");
          } catch (e) {
            pushToast("warn", `default printer load failed: ${e}`);
          }
        }
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
      if ((camera as THREE.PerspectiveCamera).isPerspectiveCamera) {
        const p = camera as THREE.PerspectiveCamera;
        p.aspect = aspect;
        p.updateProjectionMatrix();
      } else {
        const ortho = camera as THREE.OrthographicCamera;
        const half = (ortho.top - ortho.bottom) * 0.5;
        ortho.left = -half * aspect;
        ortho.right = half * aspect;
        ortho.updateProjectionMatrix();
      }
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
      const hits = raycaster.intersectObjects(
        mirror.objectGroup.children,
        false,
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

    // ---- "Frame all" + projection toggle expose via window for the
    //      surrounding React UI to call into. Refs would be cleaner but
    //      this lets the buttons live in App.tsx without prop drilling
    //      through every level. For Phase 2 MVP, fine.
    (window as unknown as N3OViewportApi).__n3o_viewport = {
      frameAll: () => {
        const box = new THREE.Box3();
        let any = false;
        mirror.objectGroup.traverse((node) => {
          if ((node as THREE.Mesh).isMesh) {
            const m = node as THREE.Mesh;
            m.updateMatrixWorld(true);
            if (m.geometry.boundingBox) {
              const b = m.geometry.boundingBox.clone().applyMatrix4(m.matrixWorld);
              box.union(b);
              any = true;
            }
          }
        });
        if (!any && mirror.bed) {
          box.set(
            new THREE.Vector3(...mirror.bed.extents.min),
            new THREE.Vector3(...mirror.bed.extents.max),
          );
          any = true;
        }
        if (any) {
          frameBox(camera, controls, box, aspect);
        }
      },
      switchProjection: (mode) => {
        const target = controls.target.clone();
        const offset = camera.position.clone().sub(target);
        if (mode === "Orthographic" && (camera as THREE.PerspectiveCamera).isPerspectiveCamera) {
          camera = makeOrthographicCamera(aspect, 200);
        } else if (mode === "Perspective" && !(camera as THREE.PerspectiveCamera).isPerspectiveCamera) {
          camera = makePerspectiveCamera(aspect);
        } else {
          return;
        }
        camera.position.copy(target).add(offset);
        camera.lookAt(target);
        controls.dispose();
        controls = makeControls(camera, renderer.domElement);
        controls.target.copy(target);
      },
    };

    return () => {
      cancelAnimationFrame(raf);
      renderer.domElement.removeEventListener("click", onClick);
      window.removeEventListener("keydown", onKeyDown);
      resizeObserver.disconnect();
      detachToastsListener();
      if (detachBridge) void detachBridge();
      gizmo.dispose();
      controls.dispose();
      mirror.clear();
      renderer.dispose();
      if (renderer.domElement.parentElement === container) {
        container.removeChild(renderer.domElement);
      }
      delete (window as unknown as N3OViewportApi).__n3o_viewport;
    };

    function pushToast(level: ToastMessage["level"], text: string) {
      const id = nextToastId++;
      setToasts((prev) => [...prev, { id, level, text }]);
      setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, 4000);
    }
    // selectedIds is used elsewhere; no warn-suppress needed.
  }, []);

  const switchProjection = (mode: ProjectionMode) => {
    setProjection(mode);
    (window as unknown as N3OViewportApi).__n3o_viewport?.switchProjection(mode);
  };

  return (
    <div className="relative w-full h-full">
      <div ref={containerRef} className="w-full h-full" />
      <div className="absolute top-2 left-2 flex flex-col gap-1 pointer-events-none">
        <div className="flex gap-2 pointer-events-auto">
          <button
            type="button"
            className="bg-neutral-800/90 text-neutral-100 text-xs px-3 py-1 rounded shadow"
            onClick={() => (window as unknown as N3OViewportApi).__n3o_viewport?.frameAll()}
          >
            Frame all
          </button>
          <button
            type="button"
            className="bg-neutral-800/90 text-neutral-100 text-xs px-3 py-1 rounded shadow"
            onClick={() => {
              // 20 mm cube via the same path the future library
              // panel will use. PR-2-7's primitive dedup means
              // re-clicking shares one MeshId with multiple objects.
              void invoke("scene_object_add_from_primitive", {
                kind: "Cube",
                params: {
                  width: 20.0,
                  depth: 20.0,
                  height: 20.0,
                  radius: 0.0,
                  radial_segments: 0,
                },
              });
            }}
            title="Add a 20 mm cube at plate center"
          >
            + Cube
          </button>
          <button
            type="button"
            className="bg-neutral-800/90 text-neutral-100 text-xs px-3 py-1 rounded shadow"
            onClick={() => {
              void (async () => {
                const picked = await openDialog({
                  multiple: false,
                  filters: [
                    {
                      name: "Mesh / project",
                      extensions: ["stl", "obj", "3mf"],
                    },
                  ],
                });
                if (typeof picked !== "string") return;
                const lower = picked.toLowerCase();
                try {
                  if (lower.endsWith(".3mf")) {
                    await invoke("scene_load_3mf", { path: picked });
                  } else {
                    await invoke("scene_load_mesh_from_path", {
                      path: picked,
                    });
                  }
                } catch (err) {
                  alert(`load failed: ${err}`);
                }
              })();
            }}
            title="Open a .stl / .obj / .3mf file"
          >
            Load…
          </button>
          <div className="bg-neutral-800/90 text-neutral-100 text-xs rounded shadow flex overflow-hidden">
            {(["Perspective", "Orthographic"] as ProjectionMode[]).map((mode) => (
              <button
                key={mode}
                type="button"
                className={`px-3 py-1 ${
                  projection === mode
                    ? "bg-neutral-700"
                    : "hover:bg-neutral-700/60"
                }`}
                onClick={() => switchProjection(mode)}
              >
                {mode === "Perspective" ? "P" : "O"}
              </button>
            ))}
          </div>
        </div>
        {selectedIds.length > 0 && (
          <div className="flex gap-2 pointer-events-auto">
            <div className="bg-neutral-800/90 text-neutral-100 text-xs rounded shadow flex overflow-hidden">
              {(
                [
                  ["None", "·"],
                  ["Translate", "T"],
                  ["Rotate", "R"],
                  ["Scale", "S"],
                ] as [GizmoMode, string][]
              ).map(([mode, label]) => (
                <button
                  key={mode}
                  type="button"
                  className={`px-3 py-1 ${
                    gizmoMode === mode
                      ? "bg-neutral-700"
                      : "hover:bg-neutral-700/60"
                  }`}
                  onClick={() => {
                    void invoke("scene_gizmo_set", {
                      gizmo: { mode, pivot: null },
                    });
                  }}
                  title={`Gizmo: ${mode}`}
                >
                  {label}
                </button>
              ))}
            </div>
            <div className="bg-neutral-800/90 text-neutral-100 text-xs px-3 py-1 rounded shadow">
              {selectedIds.length} selected · Del to remove
            </div>
          </div>
        )}
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

interface N3OViewportApi {
  __n3o_viewport?: {
    frameAll: () => void;
    switchProjection: (mode: ProjectionMode) => void;
  };
}
