// Network-first: cachea SOLO la interfaz; /api/* nunca pasa por la caché.
const CACHE = 'martino-v2';
const SHELL = ['/', '/index.html', '/styles.css', '/martino-web.js', '/martino-web_bg.wasm', '/manifest.webmanifest',
  '/icons/icon.svg', '/icons/icon-192.png', '/icons/icon-512.png', '/icons/maskable-512.png', '/icons/apple-touch-icon.png'];

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});
self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys().then((ks) => Promise.all(ks.filter((k) => k !== CACHE).map((k) => caches.delete(k)))).then(() => self.clients.claim()));
});
self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);
  if (e.request.method !== 'GET' || url.origin !== location.origin || url.pathname.startsWith('/api/')) return;
  e.respondWith(
    fetch(e.request)
      .then((r) => {
        // El clon se saca antes de devolver la respuesta: después su cuerpo ya se leyó y la copia nunca se actualizaba.
        if (r.ok && SHELL.includes(url.pathname)) { const copy = r.clone(); caches.open(CACHE).then((c) => c.put(url.pathname, copy)); }
        return r;
      })
      .catch(() => caches.match(url.pathname).then((r) => r || caches.match('/index.html')))
  );
});
