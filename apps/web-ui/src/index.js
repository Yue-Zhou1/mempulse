import * as THREE from 'https://unpkg.com/three@0.164.1/build/three.module.js';

const mount = document.getElementById('scene');

const scene = new THREE.Scene();
scene.fog = new THREE.Fog(0x020617, 2, 18);

const camera = new THREE.PerspectiveCamera(
  65,
  window.innerWidth / window.innerHeight,
  0.1,
  100,
);
camera.position.set(0, 1.2, 4);

const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
renderer.setSize(window.innerWidth, window.innerHeight);
mount.appendChild(renderer.domElement);

const keyLight = new THREE.DirectionalLight(0x7dd3fc, 1.3);
keyLight.position.set(2, 3, 4);
scene.add(keyLight);

const fillLight = new THREE.AmbientLight(0x94a3b8, 0.5);
scene.add(fillLight);

const geometry = new THREE.IcosahedronGeometry(1, 0);
const material = new THREE.MeshStandardMaterial({
  color: 0x38bdf8,
  emissive: 0x0ea5e9,
  emissiveIntensity: 0.25,
  metalness: 0.3,
  roughness: 0.3,
  wireframe: true,
});
const mesh = new THREE.Mesh(geometry, material);
scene.add(mesh);

const ring = new THREE.Mesh(
  new THREE.TorusGeometry(1.8, 0.03, 16, 120),
  new THREE.MeshBasicMaterial({ color: 0xf97316 }),
);
ring.rotation.x = Math.PI / 2;
scene.add(ring);

function onResize() {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
}
window.addEventListener('resize', onResize);

function animate() {
  mesh.rotation.x += 0.004;
  mesh.rotation.y += 0.006;
  ring.rotation.z += 0.002;
  renderer.render(scene, camera);
  requestAnimationFrame(animate);
}

animate();
