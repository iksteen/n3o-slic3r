// BuildPlate.jsx — three.js viewport for the slicer.
// Renders a build plate with grid, drag-arrangeable primitive meshes,
// and orbit controls. Selection syncs with parent via props.

const { useEffect, useRef, useState, useMemo } = React;

// Geometry factory for our object library
function makeGeometry(THREE, kind, params = {}) {
  switch (kind) {
    case "cube":
      return new THREE.BoxGeometry(params.size || 20, params.size || 20, params.size || 20);
    case "cylinder":
      return new THREE.CylinderGeometry(params.r || 12, params.r || 12, params.h || 30, 48);
    case "sphere":
      return new THREE.SphereGeometry(params.r || 14, 32, 24);
    case "cone":
      return new THREE.ConeGeometry(params.r || 14, params.h || 30, 32);
    case "torus":
      return new THREE.TorusGeometry(params.r || 12, params.tube || 4, 16, 48);
    case "benchy": {
      // crude "boat-shaped" placeholder hull (not the copyrighted geometry)
      const shape = new THREE.Shape();
      shape.moveTo(-30, -10);
      shape.lineTo(30, -10);
      shape.lineTo(25, 8);
      shape.lineTo(-25, 8);
      shape.lineTo(-30, -10);
      const geom = new THREE.ExtrudeGeometry(shape, { depth: 18, bevelEnabled: true, bevelSize: 1.5, bevelThickness: 1, bevelSegments: 2 });
      geom.rotateX(-Math.PI / 2);
      geom.translate(0, 0, 0);
      return geom;
    }
    case "calicube": {
      const g = new THREE.BoxGeometry(20, 20, 20);
      return g;
    }
    case "temptower": {
      const merged = new THREE.BufferGeometry();
      // approximate via stacked boxes — we just return a tall box; segments drawn via shader would be fancier
      return new THREE.BoxGeometry(28, 80, 18);
    }
    case "stl_mount":
      return new THREE.BoxGeometry(45, 12, 30);
    case "stl_bracket":
      return new THREE.BoxGeometry(36, 28, 8);
    default:
      return new THREE.BoxGeometry(20, 20, 20);
  }
}

function BuildPlate({
  objects, setObjects,
  selectedId, setSelectedId,
  plateSize, // [x, y]
  filaments,
  onCameraReset,
}) {
  const mountRef = useRef(null);
  const stateRef = useRef({});
  const [draggingId, setDraggingId] = useState(null);

  // Setup once
  useEffect(() => {
    if (!window.THREE) return;
    const THREE = window.THREE;
    const mount = mountRef.current;
    if (!mount) return;

    const scene = new THREE.Scene();
    scene.background = null;

    const w = mount.clientWidth || 800;
    const h = mount.clientHeight || 600;

    const camera = new THREE.PerspectiveCamera(40, w / h, 1, 4000);
    camera.position.set(280, 260, 320);
    camera.lookAt(0, 30, 0);

    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setSize(w, h);
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    mount.appendChild(renderer.domElement);

    // Lights
    const hemi = new THREE.HemisphereLight(0xffffff, 0xe6e8ec, 0.6);
    scene.add(hemi);
    const dir = new THREE.DirectionalLight(0xffffff, 0.9);
    dir.position.set(120, 220, 80);
    dir.castShadow = true;
    dir.shadow.mapSize.set(1024, 1024);
    dir.shadow.camera.left = -200;
    dir.shadow.camera.right = 200;
    dir.shadow.camera.top = 200;
    dir.shadow.camera.bottom = -200;
    scene.add(dir);
    const fill = new THREE.DirectionalLight(0xffffff, 0.25);
    fill.position.set(-200, 100, -120);
    scene.add(fill);

    // Build plate (group)
    const plateGroup = new THREE.Group();
    scene.add(plateGroup);

    const buildPlate = (sizeX, sizeY) => {
      while (plateGroup.children.length) plateGroup.remove(plateGroup.children[0]);

      // Base plate
      const plateGeom = new THREE.BoxGeometry(sizeX, 4, sizeY);
      const plateMat = new THREE.MeshStandardMaterial({
        color: 0x1f2228,
        roughness: 0.85,
        metalness: 0.05,
      });
      const plate = new THREE.Mesh(plateGeom, plateMat);
      plate.position.y = -2;
      plate.receiveShadow = true;
      plateGroup.add(plate);

      // Bevel edge — thin lighter cap
      const capGeom = new THREE.BoxGeometry(sizeX, 0.4, sizeY);
      const capMat = new THREE.MeshStandardMaterial({ color: 0x2a2e36, roughness: 0.7 });
      const cap = new THREE.Mesh(capGeom, capMat);
      cap.position.y = 0.21;
      plateGroup.add(cap);

      // Grid
      const grid = new THREE.GridHelper(Math.max(sizeX, sizeY), Math.max(sizeX, sizeY) / 10, 0x4a505a, 0x35393f);
      grid.material.opacity = 0.5; grid.material.transparent = true;
      grid.position.y = 0.42;
      // crop to plate (we use scale trick — grid is square so we let it overflow, masked by plate edges? skip mask, use second grid)
      plateGroup.add(grid);

      // Plate outline
      const outlineGeom = new THREE.EdgesGeometry(plateGeom);
      const outlineMat = new THREE.LineBasicMaterial({ color: 0x6a727f });
      const outline = new THREE.LineSegments(outlineGeom, outlineMat);
      outline.position.y = -2;
      plateGroup.add(outline);

      // Origin marker
      const originGeom = new THREE.RingGeometry(2, 3, 16);
      const originMat = new THREE.MeshBasicMaterial({ color: 0xff5566, side: THREE.DoubleSide });
      const origin = new THREE.Mesh(originGeom, originMat);
      origin.rotation.x = -Math.PI / 2;
      origin.position.set(-sizeX / 2 + 5, 0.5, sizeY / 2 - 5);
      plateGroup.add(origin);

      // X/Y axis labels (just colored lines)
      const xAxisMat = new THREE.LineBasicMaterial({ color: 0xff5566 });
      const xAxisGeom = new THREE.BufferGeometry().setFromPoints([
        new THREE.Vector3(-sizeX / 2 + 2, 0.5, sizeY / 2 - 5),
        new THREE.Vector3(-sizeX / 2 + 22, 0.5, sizeY / 2 - 5),
      ]);
      plateGroup.add(new THREE.Line(xAxisGeom, xAxisMat));
      const yAxisMat = new THREE.LineBasicMaterial({ color: 0x66cc88 });
      const yAxisGeom = new THREE.BufferGeometry().setFromPoints([
        new THREE.Vector3(-sizeX / 2 + 2, 0.5, sizeY / 2 - 5),
        new THREE.Vector3(-sizeX / 2 + 2, 0.5, sizeY / 2 - 25),
      ]);
      plateGroup.add(new THREE.Line(yAxisGeom, yAxisMat));
    };

    buildPlate(plateSize[0], plateSize[1]);

    // Object group
    const objectGroup = new THREE.Group();
    scene.add(objectGroup);

    // Orbit controls (minimal hand-rolled, since we may not have OrbitControls)
    const ctrl = {
      rotating: false, panning: false,
      lastX: 0, lastY: 0,
      theta: Math.atan2(camera.position.x, camera.position.z),
      phi: Math.acos(camera.position.y / camera.position.length()),
      radius: camera.position.length(),
      target: new THREE.Vector3(0, 30, 0),
    };

    const applyCamera = () => {
      const sinPhi = Math.sin(ctrl.phi);
      camera.position.x = ctrl.target.x + ctrl.radius * sinPhi * Math.sin(ctrl.theta);
      camera.position.z = ctrl.target.z + ctrl.radius * sinPhi * Math.cos(ctrl.theta);
      camera.position.y = ctrl.target.y + ctrl.radius * Math.cos(ctrl.phi);
      camera.lookAt(ctrl.target);
    };
    applyCamera();

    // Raycaster for object selection + drag
    const raycaster = new THREE.Raycaster();
    const mouse = new THREE.Vector2();
    const dragPlane = new THREE.Plane(new THREE.Vector3(0, 1, 0), 0);
    const dragOffset = new THREE.Vector3();
    let dragMesh = null;

    const getMouse = (e) => {
      const rect = renderer.domElement.getBoundingClientRect();
      mouse.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      mouse.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
    };

    const intersectGround = () => {
      const target = new THREE.Vector3();
      raycaster.setFromCamera(mouse, camera);
      raycaster.ray.intersectPlane(dragPlane, target);
      return target;
    };

    const onPointerDown = (e) => {
      getMouse(e);
      raycaster.setFromCamera(mouse, camera);
      const intersects = raycaster.intersectObjects(objectGroup.children, false);
      if (e.button === 0 && intersects.length > 0) {
        const mesh = intersects[0].object;
        dragMesh = mesh;
        setSelectedId(mesh.userData.id);
        setDraggingId(mesh.userData.id);
        const ground = intersectGround();
        dragOffset.set(mesh.position.x - ground.x, 0, mesh.position.z - ground.z);
        renderer.domElement.style.cursor = "grabbing";
      } else if (e.button === 0) {
        setSelectedId(null);
        ctrl.rotating = true;
        ctrl.lastX = e.clientX; ctrl.lastY = e.clientY;
      } else if (e.button === 1 || e.button === 2) {
        ctrl.panning = true;
        ctrl.lastX = e.clientX; ctrl.lastY = e.clientY;
      }
    };
    const onPointerMove = (e) => {
      getMouse(e);
      if (dragMesh) {
        const ground = intersectGround();
        const halfX = plateSize[0] / 2;
        const halfY = plateSize[1] / 2;
        const newX = Math.max(-halfX, Math.min(halfX, ground.x + dragOffset.x));
        const newZ = Math.max(-halfY, Math.min(halfY, ground.z + dragOffset.z));
        dragMesh.position.x = newX;
        dragMesh.position.z = newZ;
        // sync to state
        setObjects(prev => prev.map(o =>
          o.id === dragMesh.userData.id ? { ...o, x: newX, y: newZ } : o
        ));
      } else if (ctrl.rotating) {
        const dx = e.clientX - ctrl.lastX;
        const dy = e.clientY - ctrl.lastY;
        ctrl.theta -= dx * 0.006;
        ctrl.phi -= dy * 0.006;
        ctrl.phi = Math.max(0.15, Math.min(Math.PI / 2 - 0.05, ctrl.phi));
        ctrl.lastX = e.clientX; ctrl.lastY = e.clientY;
        applyCamera();
      } else if (ctrl.panning) {
        const dx = e.clientX - ctrl.lastX;
        const dy = e.clientY - ctrl.lastY;
        // pan in world space
        const panSpeed = ctrl.radius * 0.0015;
        const right = new THREE.Vector3(Math.cos(ctrl.theta), 0, -Math.sin(ctrl.theta));
        const up = new THREE.Vector3(0, 1, 0);
        ctrl.target.addScaledVector(right, -dx * panSpeed);
        ctrl.target.addScaledVector(up, dy * panSpeed);
        ctrl.lastX = e.clientX; ctrl.lastY = e.clientY;
        applyCamera();
      } else {
        // hover: change cursor when over an object
        raycaster.setFromCamera(mouse, camera);
        const intersects = raycaster.intersectObjects(objectGroup.children, false);
        renderer.domElement.style.cursor = intersects.length > 0 ? "grab" : "default";
      }
    };
    const onPointerUp = (e) => {
      if (dragMesh) {
        dragMesh = null;
        setDraggingId(null);
        renderer.domElement.style.cursor = "default";
      }
      ctrl.rotating = false;
      ctrl.panning = false;
    };
    const onWheel = (e) => {
      e.preventDefault();
      const delta = e.deltaY > 0 ? 1.1 : 0.9;
      ctrl.radius = Math.max(80, Math.min(900, ctrl.radius * delta));
      applyCamera();
    };
    const onContext = (e) => e.preventDefault();

    renderer.domElement.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    renderer.domElement.addEventListener("wheel", onWheel, { passive: false });
    renderer.domElement.addEventListener("contextmenu", onContext);

    // Resize
    const onResize = () => {
      const w = mount.clientWidth;
      const h = mount.clientHeight;
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      renderer.setSize(w, h);
    };
    const ro = new ResizeObserver(onResize);
    ro.observe(mount);

    // Animate
    let raf;
    const animate = () => {
      raf = requestAnimationFrame(animate);
      renderer.render(scene, camera);
    };
    animate();

    // expose
    stateRef.current = {
      THREE, scene, camera, renderer, plateGroup, objectGroup, ctrl, applyCamera, buildPlate,
    };

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      renderer.domElement.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      renderer.domElement.removeEventListener("wheel", onWheel);
      renderer.domElement.removeEventListener("contextmenu", onContext);
      try { mount.removeChild(renderer.domElement); } catch {}
      renderer.dispose();
    };
  }, []);

  // Rebuild plate if size changes
  useEffect(() => {
    const s = stateRef.current;
    if (s && s.buildPlate) s.buildPlate(plateSize[0], plateSize[1]);
  }, [plateSize[0], plateSize[1]]);

  // Sync objects to scene
  useEffect(() => {
    const s = stateRef.current;
    if (!s || !s.THREE) return;
    const { THREE, objectGroup } = s;

    // Reconcile: remove meshes whose ids vanished
    const idsInState = new Set(objects.map(o => o.id));
    const meshesToRemove = [];
    objectGroup.children.forEach(m => {
      if (!idsInState.has(m.userData.id)) meshesToRemove.push(m);
    });
    meshesToRemove.forEach(m => objectGroup.remove(m));

    // Add or update
    objects.forEach(obj => {
      let mesh = objectGroup.children.find(m => m.userData.id === obj.id);
      const fil = filaments.find(f => f.id === obj.filamentId) || filaments[0];
      const color = new THREE.Color(fil.color || "#7AA2D9");
      const isSelected = obj.id === selectedId;
      const emissive = isSelected ? new THREE.Color("#FFFFFF").multiplyScalar(0.08) : new THREE.Color(0,0,0);

      if (!mesh) {
        const geom = makeGeometry(THREE, obj.kind);
        // size it so it sits on the plate
        geom.computeBoundingBox();
        const bb = geom.boundingBox;
        const minY = bb.min.y;
        geom.translate(0, -minY, 0);
        const mat = new THREE.MeshStandardMaterial({
          color, roughness: 0.55, metalness: 0.05, emissive,
        });
        mesh = new THREE.Mesh(geom, mat);
        mesh.castShadow = true;
        mesh.receiveShadow = true;
        mesh.userData.id = obj.id;
        mesh.userData.kind = obj.kind;
        // Add outline (using EdgesGeometry as child)
        const edges = new THREE.EdgesGeometry(geom, 30);
        const lineMat = new THREE.LineBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.0 });
        const wire = new THREE.LineSegments(edges, lineMat);
        wire.userData.isOutline = true;
        mesh.add(wire);
        objectGroup.add(mesh);
      }
      mesh.position.set(obj.x, 0, obj.y);
      if (obj.rotZ !== undefined) mesh.rotation.y = obj.rotZ;
      if (mesh.material) {
        mesh.material.color.copy(color);
        mesh.material.emissive.copy(emissive);
        if (isSelected) {
          mesh.material.emissiveIntensity = 1;
        } else {
          mesh.material.emissiveIntensity = 0;
        }
      }
      // outline
      const wire = mesh.children.find(c => c.userData.isOutline);
      if (wire) {
        wire.material.opacity = isSelected ? 0.9 : 0.0;
        wire.material.color.set(isSelected ? 0xffffff : 0xffffff);
      }
    });
  }, [objects, selectedId, filaments, plateSize[0], plateSize[1]]);

  // Drop handling (from object library drag)
  const onDragOver = (e) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    mountRef.current?.parentElement?.classList.add("drop-target");
  };
  const onDragLeave = () => {
    mountRef.current?.parentElement?.classList.remove("drop-target");
  };
  const onDrop = (e) => {
    e.preventDefault();
    mountRef.current?.parentElement?.classList.remove("drop-target");
    try {
      const payload = JSON.parse(e.dataTransfer.getData("application/json"));
      const s = stateRef.current;
      if (!s) return;
      // compute drop world position on the ground plane
      const rect = s.renderer.domElement.getBoundingClientRect();
      const mouse = new s.THREE.Vector2(
        ((e.clientX - rect.left) / rect.width) * 2 - 1,
        -((e.clientY - rect.top) / rect.height) * 2 + 1
      );
      const ray = new s.THREE.Raycaster();
      ray.setFromCamera(mouse, s.camera);
      const target = new s.THREE.Vector3();
      ray.ray.intersectPlane(new s.THREE.Plane(new s.THREE.Vector3(0, 1, 0), 0), target);
      const halfX = plateSize[0] / 2;
      const halfY = plateSize[1] / 2;
      const x = Math.max(-halfX, Math.min(halfX, target.x));
      const y = Math.max(-halfY, Math.min(halfY, target.z));

      const id = `obj_${Date.now()}_${Math.floor(Math.random() * 999)}`;
      setObjects(prev => [...prev, {
        id,
        name: payload.name,
        kind: payload.kind,
        x, y, rotZ: 0,
        filamentId: payload.filamentId || filaments[0].id,
        overrides: {},
      }]);
      setSelectedId(id);
    } catch (err) {
      console.warn("drop failed", err);
    }
  };

  // Camera reset
  useEffect(() => {
    if (!onCameraReset) return;
    onCameraReset.current = () => {
      const s = stateRef.current;
      if (!s) return;
      s.ctrl.theta = Math.atan2(280, 320);
      s.ctrl.phi = Math.PI / 2.6;
      s.ctrl.radius = 480;
      s.ctrl.target.set(0, 30, 0);
      s.applyCamera();
    };
  }, [onCameraReset]);

  return (
    <div
      ref={mountRef}
      style={{ position: "absolute", inset: 0 }}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    />
  );
}

window.BuildPlate = BuildPlate;
