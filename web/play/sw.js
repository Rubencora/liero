const CACHE_NAME = 'liero-v1';
const ASSETS = [
    '/play/',
    '/play/index.html',
    '/play/pkg/liero_web.js',
    '/play/pkg/liero_web_bg.wasm',
    '/play/pkg/liero_web_bg.wasm.d.ts'
];

self.addEventListener('install', e => {
    e.waitUntil(
        caches.open(CACHE_NAME).then(cache => cache.addAll(ASSETS))
    );
});

self.addEventListener('fetch', e => {
    e.respondWith(
        caches.match(e.request).then(r => r || fetch(e.request))
    );
});
