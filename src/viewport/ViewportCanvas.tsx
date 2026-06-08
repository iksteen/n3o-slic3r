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
import { onEvents } from "../state/eventRouter";
import { useEffect, useRef, useState, type ReactNode } from "react";
import * as THREE from "three";
import type { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  initialFrameForBed,
  makeControls,
  makePerspectiveCamera,
} from "./cameraControls";
import {
  attachEventBridge,
  tauriMeshBufferProvider,
  tauriMeshPaintProvider,
} from "./eventBridge";
import { createGizmo, type GizmoApi } from "./gizmo";
import { SceneMirror } from "./sceneMirror";
import { createTowerOverlay } from "./towerOverlay";
import { getCachedTowerMesh, onTowerMeshCacheChange } from "./towerMeshCache";
import { pushLog } from "../logging/logStore";
import type { BedMesh, GizmoMode, ObjectId, TowerGeometry } from "./types";

interface ToastMessage {
  id: number;
  level: "info" | "warn" | "error";
  text: string;
}

let nextToastId = 1;

// Transform-gizmo mode icons, ported from the design mockup
// (docs/dev/design/app.jsx vp-toolbar): move arrows, a rotate arc, and a
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

  // Engine "Auto orient" of the current selection. The backend orients the
  // selection as one rigid unit (combined mesh → one rotation about the shared
  // center), so a group/assembly keeps its arrangement. Errors surface as a
  // toast rather than failing silently.
  const runAutoOrient = () => {
    const notify = (level: ToastMessage["level"], text: string) => {
      const id = nextToastId++;
      setToasts((prev) => [...prev, { id, level, text }]);
      setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 4000);
      pushLog(level, text); // parity with pushToast — keep it in the error console
    };
    const ids = mirrorRef.current?.selectedIds() ?? [];
    if (ids.length === 0) {
      notify("info", "Select an object to auto-orient.");
      return;
    }
    void invoke("scene_object_auto_orient", { ids }).catch((err) =>
      notify("error", `Auto orient failed: ${err}`),
    );
  };

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
    const mirror = new SceneMirror(
      tauriMeshBufferProvider(),
      tauriMeshPaintProvider(),
    );
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
    // A gizmo drag ends with a pointerup the browser also turns into a
    // `click` on the canvas. Swallow exactly that click so releasing a
    // handle doesn't fall through to click-to-select (which would
    // deselect, or grab whatever sits behind the released handle).
    let swallowGizmoClick = false;
    gizmo.controls.addEventListener("dragging-changed", (ev) => {
      const dragging = (ev as { value: boolean }).value;
      controls.enabled = !dragging;
      if (!dragging) swallowGizmoClick = true;
    });
    gizmoRef.current = gizmo;

    // ---- Priming-tower overlay ----------------------------------------
    //
    // Draggable on the bed; position/footprint come from the backend
    // `plate_tower_geometry` (cascade-resolved + project overrides),
    // refreshed whenever the active plate, its bed, or any override
    // changes. Two representations: the predicted box (pre-slice / while
    // dragging) and the exact mesh from the last slice. The real mesh is
    // kept across drags (just re-placed) and only goes stale when the
    // plate's material count diverges from the count it was sliced at.
    const tower = createTowerOverlay();
    scene.add(tower.group);
    tower.hide();
    let towerGeom: TowerGeometry | null = null;
    let towerBedZ = 0;
    const fmtCoord = (v: number): string => (Math.round(v * 10) / 10).toString();
    // Valid range for the tower corner so its footprint stays on the bed.
    // The sliced mesh carries the true (asymmetric width × depth + brim)
    // extent; the predicted box falls back to a square width × width + brim.
    // Extents are relative to the corner (x, y), so the corner range is
    // [bed.min - extentMin, bed.max - extentMax] per axis.
    const clampTowerCorner = (
      x: number,
      y: number,
      geom: TowerGeometry,
      fp: { minX: number; minY: number; maxX: number; maxY: number } | null,
      bed: BedMesh,
    ): { x: number; y: number } => {
      // Footprint extents are axis-aligned (the mesh bbox / the square box),
      // so this clamp assumes the tower is not rotated. Both MVP printers
      // default wipe_tower_rotation_angle to 0; a nonzero rotation would need
      // the rotated AABB here. (Re-add when a rotation UI lands.)
      const b = geom.brim;
      const minLX = fp ? fp.minX : -b;
      const maxLX = fp ? fp.maxX : geom.width + b;
      const minLY = fp ? fp.minY : -b;
      const maxLY = fp ? fp.maxY : geom.width + b;
      const loX = bed.extents.min[0] - minLX;
      const hiX = bed.extents.max[0] - maxLX;
      const loY = bed.extents.min[1] - minLY;
      const hiY = bed.extents.max[1] - maxLY;
      return {
        x: Math.min(Math.max(x, loX), Math.max(loX, hiX)),
        y: Math.min(Math.max(y, loY), Math.max(loY, hiY)),
      };
    };
    const refreshTower = async (): Promise<void> => {
      const plateId = mirror.activePlateIdOrNull();
      if (plateId == null) {
        towerGeom = null;
        tower.hide();
        return;
      }
      try {
        const geom = await invoke<TowerGeometry | null>("plate_tower_geometry", {
          plateId,
        });
        if (!geom) {
          // No tower (single-material / unbound).
          towerGeom = null;
          tower.hide();
          return;
        }
        // The active plate may have changed while the query was in flight
        // (refreshTower fires on ActivePlateChanged + override/object/material
        // events, which interleave). Bail rather than clamp `geom` against the
        // *new* plate's bed or persist an override to a stale plate — the
        // switch fired its own refresh.
        if (plateId !== mirror.activePlateIdOrNull()) return;
        const bed = mirror.activePlate()?.bed ?? null;
        towerBedZ = bed ? bed.extents.min[2] : 0;
        towerGeom = geom;
        // The exact sliced mesh (module-cached, survives remounts) is shown
        // while it still matches the plate — a drag only moves the tower, so
        // the mesh stays valid and is just re-placed. A material-count change
        // reshapes it, and a printer rebind reshapes it without re-slicing
        // (no fresh mesh arrives), so either divergence falls back to the box.
        const cached = getCachedTowerMesh(plateId);
        if (
          cached &&
          cached.materialCount === geom.material_count &&
          cached.printerInstanceId === geom.printer_instance_id
        ) {
          tower.showMesh(cached.mesh, geom, towerBedZ);
        } else {
          tower.showBox(geom, towerBedZ);
        }
        // Re-clamp on-bed for the *current* footprint and persist the
        // correction: when a material-count change drops the small sliced
        // mesh back to the square predicted box, the inherited corner can
        // leave the footprint poking off the edge — keep view + override +
        // slice in agreement.
        if (bed) {
          const c = clampTowerCorner(geom.x, geom.y, geom, tower.meshFootprint(), bed);
          if (c.x !== geom.x || c.y !== geom.y) {
            // Always show the clamped (on-bed) position.
            towerGeom = { ...geom, x: c.x, y: c.y };
            tower.place(towerGeom, towerBedZ);
            // Persist only when the *rounded* corner actually moves. The clamp
            // boundary is a float (e.g. 255.97); fmtCoord rounds it to 0.1
            // ("256"), which the backend re-resolves to 256.0 — off-bed again —
            // so comparing raw clamp vs raw geom would re-enter via the
            // override-changed event forever (255.97→"256"→256.0→255.97→…).
            // Comparing rounded values reaches a fixed point.
            const rx = fmtCoord(c.x);
            const ry = fmtCoord(c.y);
            if (rx !== fmtCoord(geom.x)) {
              void invoke("scene_project_override_set", {
                plateId,
                key: "wipe_tower_x",
                value: rx,
              });
            }
            if (ry !== fmtCoord(geom.y)) {
              void invoke("scene_project_override_set", {
                plateId,
                key: "wipe_tower_y",
                value: ry,
              });
            }
          }
        }
      } catch {
        // Transiently-unbound plate etc. — hide rather than toast-spam.
        towerGeom = null;
        tower.hide();
      }
    };

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
        case "ObjectUpdated":
          if (evt.data.plate_id !== activeId) break;
          // A selected object's transform changed programmatically (e.g.
          // auto-orient) — the mirror has already moved the mesh; reposition
          // the gizmo to follow it (the multi-selection pivot otherwise stays
          // at the old center).
          if (mirror.selectedIds().includes(evt.data.object.id)) gizmo.resync();
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
          void refreshTower();
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
          void refreshTower();
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
        void refreshTower();
      })
      .catch((err) => {
        pushToast("error", `viewport init failed: ${err}`);
      });

    // Refresh the tower on everything that can change whether it shows or
    // where it sits: override edits (a drag commit or the settings panel),
    // a quality-profile swap (prime_tower_width), and changes to the plate's
    // material count (add/remove an object, reassign a material) — the tower
    // only exists for a multi-material plate.
    const detachTowerListeners = onEvents(
      [
        "scene:project_overrides_changed",
        "scene:user_overrides_changed",
        "scene:plate_metadata_changed",
        "scene:object_added",
        "scene:object_removed",
        "scene:material_slot_changed",
      ],
      () => {
        void refreshTower();
      },
    );

    // The exact tower mesh from a finished slice lands in the module-level
    // cache, fed by App's app-lifetime `slice:plate_finished` listener (so
    // it survives this component's unmount/remount — the app auto-switches
    // to preview the instant a slice finishes). Re-render when the cache
    // changes for the active plate; on a fresh mount, refreshTower below
    // already reads the cache.
    const detachTowerCache = onTowerMeshCacheChange((plateId) => {
      if (plateId === mirror.activePlateIdOrNull()) void refreshTower();
    });

    // ---- Resize handling ---------------------------------------------
    const onResize = () => {
      const w = container.clientWidth;
      const h = Math.max(container.clientHeight, 1);
      aspect = w / h;
      renderer.setSize(w, h, false);
      camera.aspect = aspect;
      camera.updateProjectionMatrix();
      // Repaint within this callback. setSize() resizes (and clears) the
      // drawing buffer, and ResizeObserver fires after layout but before paint;
      // without an in-callback render, that frame paints a blank canvas — which
      // shows as flicker during a continuous resize (e.g. the settings-panel
      // collapse animation), where the rAF loop's render ran before the resize.
      renderer.render(scene, camera);
    };
    onResize();
    const resizeObserver = new ResizeObserver(onResize);
    resizeObserver.observe(container);

    // ---- Click-to-select ---------------------------------------------
    const raycaster = new THREE.Raycaster();
    const pointer = new THREE.Vector2();
    const onClick = (ev: MouseEvent) => {
      if (swallowGizmoClick) {
        swallowGizmoClick = false;
        return;
      }
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
          // Modifier-click toggles (so clicking a selected object again
          // deselects it) — matching the objects panel.
          mode: additive ? "Toggle" : "Replace",
          // Canvas clicks select the whole group; the object list selects
          // individual parts.
          expandGroups: true,
        });
      }
    };
    renderer.domElement.addEventListener("click", onClick);

    // ---- Priming-tower drag (bed-plane translate) ---------------------
    //
    // Grabbing the tower box drags it across the bed; releasing commits
    // wipe_tower_x/y as project overrides (which the slice honours, so the
    // box and the print move together). The capture-phase pointerdown also
    // resets the click-swallow guard — so a gizmo drag released off-canvas
    // can't suppress the next real click — and, when a tower drag starts,
    // stops the event reaching OrbitControls.
    let towerDrag: { offsetX: number; offsetY: number; moved: boolean } | null =
      null;
    const bedPlane = new THREE.Plane();
    const hitPoint = new THREE.Vector3();
    const pointerToBed = (ev: PointerEvent): THREE.Vector3 | null => {
      const rect = renderer.domElement.getBoundingClientRect();
      pointer.x = ((ev.clientX - rect.left) / rect.width) * 2 - 1;
      pointer.y = -((ev.clientY - rect.top) / rect.height) * 2 + 1;
      raycaster.setFromCamera(pointer, camera);
      bedPlane.set(new THREE.Vector3(0, 0, 1), -towerBedZ);
      return raycaster.ray.intersectPlane(bedPlane, hitPoint)
        ? hitPoint.clone()
        : null;
    };
    const onPointerDown = (ev: PointerEvent) => {
      swallowGizmoClick = false;
      // Only start a tower drag when the tower is shown, nothing else is
      // mid-drag (a gizmo drag disables orbit), and the box is the
      // frontmost thing under the pointer.
      if (towerDrag || !towerGeom || !tower.group.visible || !controls.enabled) {
        return;
      }
      const rect = renderer.domElement.getBoundingClientRect();
      pointer.x = ((ev.clientX - rect.left) / rect.width) * 2 - 1;
      pointer.y = -((ev.clientY - rect.top) / rect.height) * 2 + 1;
      raycaster.setFromCamera(pointer, camera);
      const target = tower.dragTarget();
      if (!target) return;
      const towerHits = raycaster.intersectObject(target, false);
      if (towerHits.length === 0) return;
      const objHits = raycaster.intersectObjects(
        mirror.objectGroup.children,
        true,
      );
      if (objHits.length > 0 && objHits[0].distance < towerHits[0].distance) {
        return; // an object is in front — let the click select it instead
      }
      const p = pointerToBed(ev);
      if (!p) return;
      towerDrag = {
        offsetX: p.x - towerGeom.x,
        offsetY: p.y - towerGeom.y,
        moved: false,
      };
      controls.enabled = false;
      ev.stopPropagation();
      ev.preventDefault();
    };
    const onPointerMove = (ev: PointerEvent) => {
      if (!towerDrag || !towerGeom) return;
      const p = pointerToBed(ev);
      if (!p) return;
      let nx = p.x - towerDrag.offsetX;
      let ny = p.y - towerDrag.offsetY;
      const bed = mirror.activePlate()?.bed;
      if (bed) {
        const c = clampTowerCorner(nx, ny, towerGeom, tower.meshFootprint(), bed);
        nx = c.x;
        ny = c.y;
      }
      if (nx !== towerGeom.x || ny !== towerGeom.y) towerDrag.moved = true;
      towerGeom = { ...towerGeom, x: nx, y: ny };
      tower.place(towerGeom, towerBedZ);
    };
    const onPointerUp = () => {
      if (!towerDrag) return;
      const moved = towerDrag.moved;
      towerDrag = null;
      controls.enabled = true;
      swallowGizmoClick = true; // the release also fires a click; never select
      const plateId = mirror.activePlateIdOrNull();
      // A click without a drag shouldn't pin the tower with an override.
      if (moved && plateId != null && towerGeom) {
        void invoke("scene_project_override_set", {
          plateId,
          key: "wipe_tower_x",
          value: fmtCoord(towerGeom.x),
        });
        void invoke("scene_project_override_set", {
          plateId,
          key: "wipe_tower_y",
          value: fmtCoord(towerGeom.y),
        });
        // The override-changed event triggers refreshTower → the
        // authoritative (resolved + clamped) box settles in.
      }
    };
    renderer.domElement.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);

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
      renderer.domElement.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("keydown", onKeyDown);
      detachTowerListeners();
      detachTowerCache();
      resizeObserver.disconnect();
      detachToastsListener();
      if (detachBridge) void detachBridge();
      gizmoRef.current = null;
      gizmo.dispose();
      tower.dispose();
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
      // Toasts are transient and lost on viewport unmount; also persist them
      // in the app-wide error console so the user can review them later.
      pushLog(level, text);
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
          <div className="bg-neutral-800/90 text-neutral-100 text-xs rounded shadow flex overflow-hidden">
            <button
              type="button"
              className="px-2 py-1.5 hover:bg-neutral-700/60"
              onClick={runAutoOrient}
              title="Auto orient selection (minimize supports)"
              aria-label="Auto orient selection"
            >
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
                <path
                  d="M7 1.6v6.6M4.2 5.4 7 8.2l2.8-2.8M2.2 12h9.6"
                  stroke="currentColor"
                  strokeWidth="1.4"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </button>
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
      {/* Pinned above the console toggle (bottom:12px, ~26px tall) so the
          lowest toast clears it rather than rendering behind it. */}
      <div className="absolute bottom-12 right-3 flex flex-col gap-1 pointer-events-none">
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

