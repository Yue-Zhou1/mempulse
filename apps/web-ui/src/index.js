import * as THREE from 'https://unpkg.com/three@0.164.1/build/three.module.js';
import { resolveApiBase } from './api-base.js';

const storageKey = 'vizApiBase';
const getStoredApiBase = () => {
  try {
    return window.localStorage.getItem(storageKey);
  } catch {
    return null;
  }
};
const setStoredApiBase = (value) => {
  try {
    if (value) {
      window.localStorage.setItem(storageKey, value);
    }
  } catch {
    // Ignore storage errors for private/incognito contexts.
  }
};

const apiConfig = resolveApiBase({
  search: window.location.search,
  storedApiBase: getStoredApiBase(),
  protocol: window.location.protocol,
  hostname: window.location.hostname,
});
if (apiConfig.persistApiBase) {
  setStoredApiBase(apiConfig.persistApiBase);
}
const apiBase = apiConfig.apiBase;
const refreshIntervalMs = 2000;
const mount = document.getElementById('scene');
const timelineInput = document.getElementById('timeline');
const replayMeta = document.getElementById('replayMeta');
const featureList = document.getElementById('featureList');
const txList = document.getElementById('txList');
const apiStatus = document.getElementById('apiStatus');

const scene = new THREE.Scene();
scene.fog = new THREE.Fog(0x020617, 2, 30);

const camera = new THREE.PerspectiveCamera(
  65,
  window.innerWidth / window.innerHeight,
  0.1,
  100,
);
camera.position.set(0, 1.2, 8);

const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
renderer.setSize(window.innerWidth, window.innerHeight);
mount.appendChild(renderer.domElement);

scene.add(new THREE.AmbientLight(0x94a3b8, 0.6));
const keyLight = new THREE.DirectionalLight(0x67e8f9, 1.2);
keyLight.position.set(4, 6, 5);
scene.add(keyLight);

const graphGroup = new THREE.Group();
scene.add(graphGroup);

const replayMesh = new THREE.Mesh(
  new THREE.IcosahedronGeometry(0.8, 0),
  new THREE.MeshStandardMaterial({
    color: 0x38bdf8,
    emissive: 0x0369a1,
    emissiveIntensity: 0.45,
    wireframe: true,
  }),
);
replayMesh.position.set(0, -1.4, 0);
scene.add(replayMesh);

const replayRing = new THREE.Mesh(
  new THREE.TorusGeometry(1.5, 0.03, 16, 120),
  new THREE.MeshBasicMaterial({ color: 0xfb923c }),
);
replayRing.rotation.x = Math.PI / 2;
replayRing.position.y = -1.4;
scene.add(replayRing);

let replayFrames = [];
let propagationEdges = [];

function clearGroup(group) {
  while (group.children.length > 0) {
    const child = group.children.pop();
    if (child.geometry) child.geometry.dispose();
    if (child.material) child.material.dispose();
  }
}

function buildGraph(edges) {
  clearGroup(graphGroup);
  if (!edges.length) return;

  const peers = [...new Set(edges.flatMap((edge) => [edge.source, edge.destination]))];
  const positions = new Map();
  const radius = 2.6;

  peers.forEach((peer, idx) => {
    const angle = (idx / peers.length) * Math.PI * 2;
    positions.set(peer, new THREE.Vector3(Math.cos(angle) * radius, 0, Math.sin(angle) * radius));
  });

  const nodeGeometry = new THREE.SphereGeometry(0.12, 16, 16);
  const nodeMaterial = new THREE.MeshStandardMaterial({ color: 0xa5f3fc, emissive: 0x0891b2 });
  positions.forEach((position) => {
    const node = new THREE.Mesh(nodeGeometry, nodeMaterial);
    node.position.copy(position);
    graphGroup.add(node);
  });

  edges.forEach((edge) => {
    const source = positions.get(edge.source);
    const destination = positions.get(edge.destination);
    if (!source || !destination) return;
    const points = [source, destination];
    const lineGeometry = new THREE.BufferGeometry().setFromPoints(points);
    const intensity = Math.min(1, edge.p99_delay_ms / 100);
    const color = new THREE.Color().setHSL(0.08 + intensity * 0.55, 1.0, 0.55);
    const line = new THREE.Line(lineGeometry, new THREE.LineBasicMaterial({ color }));
    graphGroup.add(line);
  });
}

function applyReplayFrame(index) {
  if (!replayFrames.length) return;
  const clamped = Math.max(0, Math.min(index, replayFrames.length - 1));
  const frame = replayFrames[clamped];
  const pendingCount = frame.pending_count;
  const scale = Math.max(0.6, Math.min(2.4, 0.6 + pendingCount / 4000));

  replayMesh.scale.setScalar(scale);
  replayRing.scale.setScalar(scale * 0.85);

  replayMeta.textContent = `frame ${clamped + 1}/${replayFrames.length} | seq=${frame.seq_hi} | pending=${pendingCount}`;
}

function renderFeatures(features) {
  featureList.innerHTML = '';
  if (!features.length) {
    const item = document.createElement('li');
    item.textContent = 'No feature rows available';
    featureList.appendChild(item);
    return;
  }

  features.slice(0, 6).forEach((feature) => {
    const item = document.createElement('li');
    item.textContent = `${feature.protocol}/${feature.category}: ${feature.count}`;
    featureList.appendChild(item);
  });
}

function shortenHex(value, head = 10, tail = 8) {
  if (!value || value.length <= head + tail) return value;
  return `${value.slice(0, head)}...${value.slice(-tail)}`;
}

function renderTransactions(transactions) {
  txList.innerHTML = '';
  if (!transactions.length) {
    const item = document.createElement('li');
    item.textContent = 'No mempool transactions yet';
    txList.appendChild(item);
    return;
  }

  transactions.slice(0, 12).forEach((tx) => {
    const item = document.createElement('li');
    item.textContent = `${shortenHex(tx.hash)} nonce=${tx.nonce} type=${tx.tx_type} from=${shortenHex(tx.sender, 8, 6)} src=${tx.source_id}`;
    txList.appendChild(item);
  });
}

async function fetchJson(path) {
  const response = await fetch(`${apiBase}${path}`);
  if (!response.ok) {
    throw new Error(`failed ${path}: HTTP ${response.status}`);
  }
  return response.json();
}

async function loadData() {
  apiStatus.textContent = `Loading from ${apiBase} ...`;
  try {
    const [txRows, replayRows, propagationRows, featureRows] = await Promise.all([
      fetchJson('/transactions?limit=20'),
      fetchJson('/replay'),
      fetchJson('/propagation'),
      fetchJson('/features'),
    ]);

    const previousIndex = Number(timelineInput.value || 0);
    const previousLength = replayFrames.length;
    const wasAtTail = previousLength === 0 || previousIndex >= previousLength - 1;

    replayFrames = replayRows;
    propagationEdges = propagationRows;
    timelineInput.max = String(Math.max(0, replayFrames.length - 1));
    const nextIndex = wasAtTail
      ? Math.max(0, replayFrames.length - 1)
      : Math.max(0, Math.min(previousIndex, replayFrames.length - 1));
    timelineInput.value = String(nextIndex);

    buildGraph(propagationEdges);
    applyReplayFrame(nextIndex);
    renderFeatures(featureRows);
    renderTransactions(txRows);
    const lastUpdated = new Date().toLocaleTimeString();
    apiStatus.textContent = `Connected. replay=${replayFrames.length} propagation=${propagationEdges.length} tx=${txRows.length} updated=${lastUpdated}`;
  } catch (error) {
    apiStatus.textContent = `API unavailable: ${error.message}`;
  }
}

timelineInput.addEventListener('input', (event) => {
  applyReplayFrame(Number(event.target.value));
});

function onResize() {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
}
window.addEventListener('resize', onResize);

function animate() {
  replayMesh.rotation.x += 0.004;
  replayMesh.rotation.y += 0.006;
  graphGroup.rotation.y += 0.0015;
  replayRing.rotation.z += 0.002;
  renderer.render(scene, camera);
  requestAnimationFrame(animate);
}

loadData();
animate();
window.setInterval(loadData, refreshIntervalMs);
