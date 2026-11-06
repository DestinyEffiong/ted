import * as maplibregl from 'maplibre-gl'
import workerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?url'
import 'maplibre-gl/dist/maplibre-gl.css'
import './style.css'

maplibregl.setWorkerUrl(workerUrl)

type Dot = { lat: number; lng: number; zone: string | null; seconds_ago: number }
type Zone = { name: string; lat: number; lng: number; radius_m: number }

const REFRESH_MS = 5000
const PING_MS = 5000

function byId(id: string): HTMLElement {
  const el = document.getElementById(id)
  if (!el) throw new Error('missing #' + id)
  return el
}

// a random id for this phone, kept in the browser. no account needed.
function myId(): string {
  try {
    let id = localStorage.getItem('ted-id')
    if (!id) {
      id = crypto.randomUUID()
      localStorage.setItem('ted-id', id)
    }
    return id
  } catch {
    return Math.random().toString(36).slice(2) + Date.now().toString(36)
  }
}
const ID = myId()

function ago(s: number): string {
  if (s < 60) return 'just now'
  const m = Math.floor(s / 60)
  if (m < 60) return `${m} min ago`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h} h ago`
  return `${Math.floor(h / 24)} d ago`
}

// ---------- the map ----------
const map = new maplibregl.Map({
  container: 'map',
  center: [3.723, 6.8915], // [lng, lat]
  zoom: 17,
  pitch: 55,
  maxPitch: 75,
  attributionControl: { compact: true },
  style: {
    version: 8,
    glyphs: 'https://tiles.openfreemap.org/fonts/{fontstack}/{range}.pbf',
    sources: {
      sat: {
        type: 'raster',
        tiles: ['https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}'],
        tileSize: 256,
        maxzoom: 17,
        attribution: 'Imagery © Esri',
      },
      osm: { type: 'vector', url: 'https://tiles.openfreemap.org/planet' },
    },
    layers: [
      { id: 'sat', type: 'raster', source: 'sat' },
      {
        id: 'buildings',
        type: 'fill-extrusion',
        source: 'osm',
        'source-layer': 'building',
        paint: {
          'fill-extrusion-color': '#a58468',
          'fill-extrusion-height': ['coalesce', ['get', 'render_height'], 8],
          'fill-extrusion-base': ['coalesce', ['get', 'render_min_height'], 0],
          'fill-extrusion-opacity': 0.8,
        },
      },
    ],
  },
})

const empty = { type: 'FeatureCollection', features: [] } as const

// laptop mouse: drag with the left button to look around (turn and tilt), like Minecraft.
// hold Shift while dragging to move the map instead. phones are not affected.
const mapBox = map.getContainer()
let look: { x: number; y: number } | null = null
mapBox.addEventListener(
  'mousedown',
  (e) => {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey) return
    if ((e.target as HTMLElement).closest('.maplibregl-ctrl, .maplibregl-popup')) return
    e.stopPropagation()
    look = { x: e.clientX, y: e.clientY }
  },
  true,
)
window.addEventListener('mousemove', (e) => {
  if (!look) return
  const dx = e.clientX - look.x
  const dy = e.clientY - look.y
  look = { x: e.clientX, y: e.clientY }
  map.setBearing(map.getBearing() + dx * 0.4)
  map.setPitch(Math.max(0, Math.min(75, map.getPitch() - dy * 0.3)))
})
window.addEventListener('mouseup', () => (look = null))

// trackpad: two fingers moving = move the map. pinching (which arrives with Ctrl) still zooms.
mapBox.addEventListener(
  'wheel',
  (e) => {
    if (e.ctrlKey) return
    e.preventDefault()
    e.stopPropagation()
    map.panBy([e.deltaX, e.deltaY], { animate: false })
  },
  { capture: true, passive: false },
)

map.on('load', () => {
  map.addSource('zones', { type: 'geojson', data: empty })
  map.addSource('dots', { type: 'geojson', data: empty })
  map.addSource('me', { type: 'geojson', data: empty })

  // flat 2D look: hidden until you switch to 2D
  const hide = { visibility: 'none' } as const
  map.addLayer({ id: 'flat-bg', type: 'background', layout: hide, paint: { 'background-color': '#1f3d2b' } }, 'buildings')
  map.addLayer({ id: 'flat-park', type: 'fill', source: 'osm', 'source-layer': 'park', layout: hide, paint: { 'fill-color': '#2f5a3a' } }, 'buildings')
  map.addLayer({ id: 'flat-grass', type: 'fill', source: 'osm', 'source-layer': 'landcover', layout: hide, paint: { 'fill-color': '#2a5035' } }, 'buildings')
  map.addLayer({ id: 'flat-water', type: 'fill', source: 'osm', 'source-layer': 'water', layout: hide, paint: { 'fill-color': '#2b4d6b' } }, 'buildings')
  map.addLayer(
    {
      id: 'flat-roads',
      type: 'line',
      source: 'osm',
      'source-layer': 'transportation',
      layout: { ...hide, 'line-cap': 'round', 'line-join': 'round' },
      paint: { 'line-color': '#d9d2c0', 'line-width': ['interpolate', ['linear'], ['zoom'], 14, 1, 19, 7] },
    },
    'buildings',
  )

  map.addLayer({
    id: 'zone-points',
    type: 'circle',
    source: 'zones',
    minzoom: 16,
    paint: {
      'circle-radius': 4,
      'circle-color': '#ffffff',
      'circle-stroke-color': '#16160f',
      'circle-stroke-width': 1.5,
    },
  })
  map.addLayer({
    id: 'zone-labels',
    type: 'symbol',
    source: 'zones',
    minzoom: 16,
    layout: {
      'text-field': ['get', 'name'],
      'text-font': ['Noto Sans Regular'],
      'text-size': 12,
      'text-anchor': 'top',
      'text-offset': [0, 0.7],
      'text-max-width': 8,
    },
    paint: {
      'text-color': '#ffffff',
      'text-halo-color': 'rgba(0,0,0,0.75)',
      'text-halo-width': 1.6,
    },
  })

  // strangers: grey dots that fade as they get old (solid at 0 min, faint at 40 min)
  map.addLayer({
    id: 'dots',
    type: 'circle',
    source: 'dots',
    paint: {
      'circle-radius': 7,
      'circle-color': '#8d9088',
      'circle-stroke-color': '#ffffff',
      'circle-stroke-width': 2,
      'circle-opacity': ['interpolate', ['linear'], ['get', 'age'], 0, 1, 2400, 0.25],
      'circle-stroke-opacity': ['interpolate', ['linear'], ['get', 'age'], 0, 1, 2400, 0.25],
    },
  })

  // you: bright blue with a soft halo
  map.addLayer({
    id: 'me-halo',
    type: 'circle',
    source: 'me',
    paint: { 'circle-radius': 18, 'circle-color': '#2f7cf6', 'circle-opacity': 0.25 },
  })
  map.addLayer({
    id: 'me',
    type: 'circle',
    source: 'me',
    paint: {
      'circle-radius': 8,
      'circle-color': '#2f7cf6',
      'circle-stroke-color': '#ffffff',
      'circle-stroke-width': 3,
    },
  })

  map.on('click', 'dots', (e) => {
    const f = e.features?.[0]
    if (!f || f.geometry.type !== 'Point') return
    const zone = (f.properties?.zone as string) || 'on campus'
    const age = Number(f.properties?.age ?? 0)
    const [lng, lat] = f.geometry.coordinates
    new maplibregl.Popup({ closeButton: false, offset: 12 })
      .setLngLat([lng, lat])
      .setText(`Someone, ${zone}, ${ago(age)}`)
      .addTo(map)
  })
  map.on('click', 'me', () => {
    if (!mine) return
    new maplibregl.Popup({ closeButton: false, offset: 14 })
      .setLngLat([mine.lng, mine.lat])
      .setText(myZone ? `You, ${myZone}` : 'You')
      .addTo(map)
  })
  map.on('mouseenter', 'dots', () => (map.getCanvas().style.cursor = 'pointer'))
  map.on('mouseleave', 'dots', () => (map.getCanvas().style.cursor = ''))

  loadZones()
  setMe()
  refresh()
})

async function loadZones() {
  try {
    const r = await fetch('/api/zones')
    if (!r.ok) return
    const zones: Zone[] = await r.json()
    ;(map.getSource('zones') as maplibregl.GeoJSONSource | undefined)?.setData({
      type: 'FeatureCollection',
      features: zones.map((z) => ({
        type: 'Feature',
        geometry: { type: 'Point', coordinates: [z.lng, z.lat] },
        properties: { name: z.name },
      })),
    })
  } catch {
    /* server not running yet, labels just won't show */
  }
}

async function refresh() {
  if (document.hidden) return
  try {
    const r = await fetch(`/api/dots?id=${ID}`)
    if (!r.ok) return
    const dots: Dot[] = await r.json()
    ;(map.getSource('dots') as maplibregl.GeoJSONSource | undefined)?.setData({
      type: 'FeatureCollection',
      features: dots.map((d) => ({
        type: 'Feature',
        geometry: { type: 'Point', coordinates: [d.lng, d.lat] },
        properties: { zone: d.zone ?? '', age: d.seconds_ago },
      })),
    })
  } catch {
    /* ignore, try again next time */
  }
}
setInterval(refresh, REFRESH_MS)

// ---------- your own location ----------
let mine: { lat: number; lng: number } | null = null
let myZone: string | null = null
let lastPing = 0
let firstFix = true

function setMe() {
  if (!mine) return
  ;(map.getSource('me') as maplibregl.GeoJSONSource | undefined)?.setData({
    type: 'Feature',
    geometry: { type: 'Point', coordinates: [mine.lng, mine.lat] },
    properties: {},
  })
}

function sendPing() {
  if (!mine) return
  fetch('/api/ping', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id: ID, lat: mine.lat, lng: mine.lng }),
  })
    .then((r) => (r.ok ? r.json() : null))
    .then((j) => {
      if (j) myZone = j.zone ?? null
    })
    .catch(() => {})
}

let watchId: number | null = null
let locOn = true
try {
  locOn = localStorage.getItem('ted-location') !== 'off'
} catch {
  /* ignore */
}

function startWatch() {
  if (!('geolocation' in navigator) || watchId !== null) return
  firstFix = true
  lastPing = 0
  watchId = navigator.geolocation.watchPosition(
    (pos) => {
      mine = { lat: pos.coords.latitude, lng: pos.coords.longitude }
      setMe()
      if (firstFix) {
        firstFix = false
        map.easeTo({ center: [mine.lng, mine.lat], zoom: 18, duration: 1200 })
      }
      if (Date.now() - lastPing >= PING_MS) {
        lastPing = Date.now()
        sendPing()
      }
    },
    (err) => console.warn('location:', err.message),
    { enableHighAccuracy: true, maximumAge: 5000 },
  )
}

function stopWatch() {
  if (watchId !== null) navigator.geolocation.clearWatch(watchId)
  watchId = null
  mine = null
  myZone = null
  ;(map.getSource('me') as maplibregl.GeoJSONSource | undefined)?.setData({
    type: 'FeatureCollection',
    features: [],
  })
}

// the round location button, above the 3D button
const PIN = '<path d="M12 21s-7-6.2-7-11a7 7 0 0 1 14 0c0 4.8-7 11-7 11z"/><circle cx="12" cy="10" r="2.5"/>'
const pinSvg = (inner: string) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`

const locBtn = document.createElement('button')
locBtn.type = 'button'
locBtn.id = 'loc-btn'
byId('map-view').append(locBtn)

function paintLoc() {
  locBtn.classList.toggle('on', locOn)
  locBtn.innerHTML = pinSvg(locOn ? PIN : PIN + '<path d="M3 3l18 18"/>')
  locBtn.setAttribute('aria-pressed', String(locOn))
  locBtn.setAttribute('aria-label', locOn ? 'Turn location off' : 'Turn location on')
}
paintLoc()
if (locOn) startWatch()

locBtn.addEventListener('click', () => {
  locOn = !locOn
  try {
    localStorage.setItem('ted-location', locOn ? 'on' : 'off')
  } catch {
    /* ignore */
  }
  paintLoc()
  if (locOn) {
    startWatch()
    toast('Location is on.')
  } else {
    stopWatch()
    post('/api/leave', { id: ID }) // the server forgets your dot straight away
    toast('Location is off.')
  }
})

byId('locate').addEventListener('click', () => {
  if (mine) map.flyTo({ center: [mine.lng, mine.lat], zoom: 18 })
})

// ---------- 2D / SAT / 3D switch ----------
const FLAT_LAYERS = ['flat-bg', 'flat-park', 'flat-grass', 'flat-water', 'flat-roads']
const MODES = ['3D', 'SAT', '2D'] as const
let modeIndex = 0

const modeBtn = document.createElement('button')
modeBtn.type = 'button'
modeBtn.id = 'mode-btn'
modeBtn.textContent = MODES[0]
byId('map-view').append(modeBtn)

function applyMode() {
  const m = MODES[modeIndex]
  modeBtn.textContent = m
  const flat = m === '2D'
  for (const id of FLAT_LAYERS) map.setLayoutProperty(id, 'visibility', flat ? 'visible' : 'none')
  map.setLayoutProperty('sat', 'visibility', flat ? 'none' : 'visible')
  map.setLayoutProperty('buildings', 'visibility', m === 'SAT' ? 'none' : 'visible')
  map.setPaintProperty('buildings', 'fill-extrusion-color', flat ? '#c9a27a' : '#a58468')
  map.setPaintProperty(
    'buildings',
    'fill-extrusion-height',
    flat ? 2 : ['coalesce', ['get', 'render_height'], 8],
  )
  map.easeTo({ pitch: m === '3D' ? 55 : 0, bearing: m === '3D' ? map.getBearing() : 0, duration: 600 })
}
modeBtn.addEventListener('click', () => {
  modeIndex = (modeIndex + 1) % MODES.length
  applyMode()
})

// ---------- tabs ----------
const tabs = document.querySelectorAll<HTMLButtonElement>('.tab')
const views: Record<string, HTMLElement> = {
  map: byId('map-view'),
  feed: byId('feed-view'),
}

function show(name: string) {
  tabs.forEach((t) => t.classList.toggle('active', t.dataset.tab === name))
  for (const [key, el] of Object.entries(views)) el.classList.toggle('active', key === name)
  if (name === 'map') map.resize()
  if (name === 'feed') loadFeed(true)
  syncChrome()
}
tabs.forEach((t) => t.addEventListener('click', () => show(t.dataset.tab ?? 'map')))

// ---------- feed ----------
type ReplyT = {
  id: number
  parent: number | null
  text: string
  seconds_ago: number
  was_there: boolean
  likes: number
  liked: boolean
  deleted: boolean
  mine: boolean
  nick: string | null
  color: string | null
  photo: string | null
}
type PollT = { options: { label: string; votes: number }[]; total: number; mine: number | null }
type Question = {
  id: number
  text: string
  seconds_ago: number
  place: string | null
  likes: number
  liked: boolean
  poll: PollT | null
  replies: ReplyT[]
  mine: boolean
  nick: string | null
  color: string | null
  photo: string | null
}

const SVG = (inner: string, fill = 'none') =>
  `<svg viewBox="0 0 24 24" width="20" height="20" fill="${fill}" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`
const BELL_PATH = '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>'
const ICONS = {
  heart: SVG('<path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z"/>'),
  comment: SVG('<path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z"/>'),
  person:
    '<svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"><circle cx="12" cy="9" r="4"/><path d="M4 22c0-4.4 3.6-7.5 8-7.5s8 3.1 8 7.5z"/></svg>',
  bell: SVG(BELL_PATH),
  bellOff: SVG(BELL_PATH + '<path d="M3 3l18 18"/>'),
  sun: SVG('<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>'),
  moon: SVG('<path d="M21 13A9 9 0 1 1 11 3a7 7 0 0 0 10 10z"/>'),
  trash: SVG('<path d="M4 7h16"/><path d="M9 7V4h6v3"/><path d="M6 7l1 13h10l1-13"/><path d="M10 11v6M14 11v6"/>'),
}

const feedHome = byId('feed-home')
const homeScroll = byId('home-scroll')
const feedList = byId('feed-list')
const composer = byId('composer') as HTMLFormElement
const askText = byId('ask-text') as HTMLTextAreaElement
const pollToggle = byId('poll-toggle')
const pollFields = byId('poll-fields')
const addOption = byId('add-option')
const threadEl = byId('thread')
const threadScroll = byId('thread-scroll')
const threadBody = byId('thread-body')
const replyForm = byId('reply-form') as HTMLFormElement
const replyInput = byId('reply-input') as HTMLInputElement
const replyingTo = byId('replying-to')
const replyingText = byId('replying-text')
const toastEl = byId('toast')

let questions: Question[] = []
let openId: number | null = null // which thread is open
let target: number | null = null // which reply we are answering, null = the question itself
let pendingOpen: number | null = null // a thread to open as soon as the feed has loaded

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string) {
  const e = document.createElement(tag)
  if (cls) e.className = cls
  if (text !== undefined) e.textContent = text
  return e
}

let toastTimer = 0
function toast(msg: string) {
  toastEl.textContent = msg
  toastEl.hidden = false
  clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => (toastEl.hidden = true), 4500)
}

async function post(url: string, body: unknown): Promise<boolean> {
  try {
    const r = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })
    return r.ok
  } catch {
    return false
  }
}

function syncChrome() {
  document.body.classList.toggle('in-thread', openId !== null && views.feed.classList.contains('active'))
}

function avatar(f?: { color?: string | null; photo?: string | null }) {
  const a = el('div', 'avatar')
  if (f?.photo) {
    const img = document.createElement('img')
    img.src = f.photo
    img.alt = ''
    a.append(img)
  } else {
    a.innerHTML = ICONS.person
    if (f?.color) {
      a.style.background = f.color
      a.style.color = '#ffffff'
    }
  }
  return a
}
const composerAv = avatar()
byId('composer-avatar').replaceWith(composerAv)

// ---------- profile (saved on this phone, and sent to the server so others see it) ----------
const PROFILE_COLORS = ['#e0245e', '#f59e0b', '#16a34a', '#2f7cf6', '#8b5cf6']
type Profile = { nick: string; color: string | null; photo: string | null }

function loadProfile(): Profile {
  try {
    const p = JSON.parse(localStorage.getItem('ted-profile') || '{}')
    return { nick: p.nick ?? '', color: p.color ?? null, photo: p.photo ?? null }
  } catch {
    return { nick: '', color: null, photo: null }
  }
}
const profile = loadProfile()
function saveProfile() {
  try {
    localStorage.setItem('ted-profile', JSON.stringify(profile))
  } catch {
    /* storage blocked, profile just will not be remembered */
  }
}

// the round button, above the bell
const profileBtn = el('button', 'fab')
profileBtn.type = 'button'
profileBtn.id = 'profile-btn'
document.querySelector('.fabs')?.prepend(profileBtn)

// the popup
const sheet = el('div', 'p-sheet')
sheet.hidden = true
const card = el('div', 'p-card')
const wrap = el('div', 'p-wrap')
const big = el('div', 'p-avatar')
const pencil = el('button', 'p-pencil')
pencil.type = 'button'
pencil.innerHTML = SVG('<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/>')
wrap.append(big, pencil)
const file = document.createElement('input')
file.type = 'file'
file.accept = 'image/*'
file.hidden = true
const swatches = el('div', 'p-colors')
const nickLabel = el('label', 'p-label', 'Nickname')
const nick = document.createElement('input')
nick.className = 'p-input'
nick.id = 'p-nick'
nick.maxLength = 20
nickLabel.htmlFor = 'p-nick'
const done = el('button', 'p-done', 'Save')
done.type = 'button'
card.append(wrap, file, swatches, nickLabel, nick, done)
sheet.append(card)
document.body.append(sheet)

function paintAvatar(box: HTMLElement) {
  box.replaceChildren()
  box.style.background = profile.color ?? ''
  box.style.color = profile.color ? '#ffffff' : ''
  if (profile.photo) {
    const img = document.createElement('img')
    img.src = profile.photo
    img.alt = ''
    box.append(img)
  } else {
    box.innerHTML = ICONS.person
  }
}

function paintSwatches() {
  swatches.replaceChildren()
  for (const c of PROFILE_COLORS) {
    const b = el('button', 'p-dot' + (profile.color === c && !profile.photo ? ' on' : ''))
    b.type = 'button'
    b.style.background = c
    b.addEventListener('click', () => {
      profile.color = c
      profile.photo = null // a colour fills the circle, so it replaces the picture
      refreshProfile()
    })
    swatches.append(b)
  }
  // last circle: back to the default grey person
  const reset = el('button', 'p-dot p-reset' + (!profile.color && !profile.photo ? ' on' : ''))
  reset.type = 'button'
  reset.innerHTML = ICONS.person
  reset.addEventListener('click', () => {
    profile.color = null
    profile.photo = null
    refreshProfile()
  })
  swatches.append(reset)
}

function refreshProfile() {
  paintAvatar(big)
  paintAvatar(profileBtn)
  paintAvatar(composerAv)
  paintSwatches()
  saveProfile()
}

async function pushProfile() {
  await post('/api/profile', { id: ID, nick: profile.nick || null, color: profile.color, photo: profile.photo })
  loadFeed(true)
}

function closeProfile() {
  profile.nick = nick.value.trim().slice(0, 20)
  saveProfile()
  sheet.hidden = true
  pushProfile()
}

profileBtn.addEventListener('click', () => {
  nick.value = profile.nick
  refreshProfile()
  sheet.hidden = false
})
done.addEventListener('click', closeProfile)
sheet.addEventListener('click', (e) => {
  if (e.target === sheet) closeProfile()
})
pencil.addEventListener('click', () => file.click())
big.addEventListener('click', () => file.click())

// picture: cropped to a small square so it stays tiny in storage
file.addEventListener('change', () => {
  const f = file.files?.[0]
  file.value = ''
  if (!f) return
  const url = URL.createObjectURL(f)
  const img = new Image()
  img.onload = () => {
    const size = 160
    const side = Math.min(img.width, img.height)
    const c = document.createElement('canvas')
    c.width = size
    c.height = size
    c.getContext('2d')?.drawImage(img, (img.width - side) / 2, (img.height - side) / 2, side, side, 0, 0, size, size)
    URL.revokeObjectURL(url)
    profile.photo = c.toDataURL('image/jpeg', 0.8)
    refreshProfile()
  }
  img.onerror = () => URL.revokeObjectURL(url)
  img.src = url
})
paintAvatar(profileBtn)
paintAvatar(composerAv)
if (profile.nick || profile.color || profile.photo) pushProfile()

function head(secs: number, place?: string | null, wasThere?: boolean, nickName?: string | null) {
  const h = el('div', 'head')
  h.append(el('strong', undefined, nickName || '~'))
  if (wasThere) h.append(el('span', 'tag', 'was there'))
  if (place) h.append(el('span', 'place', place))
  h.append(el('span', 'muted', ago(secs)))
  return h
}

function action(kind: 'heart' | 'comment' | 'trash', count: number, on = false, click?: () => void) {
  const b = el('button', 'act' + (on ? ' on' : '') + (kind === 'trash' ? ' del' : ''))
  b.type = 'button'
  b.innerHTML = ICONS[kind]
  if (count > 0) b.append(el('span', undefined, String(count)))
  if (click) {
    b.addEventListener('click', (e) => {
      e.stopPropagation()
      click()
    })
  }
  return b
}

// which reply the next answer goes under. the box says who, and that reply is shaded
function setTarget(r: ReplyT | null) {
  target = r ? r.id : null
  replyingTo.hidden = !r
  replyingText.textContent = r ? 'Replying to ~' : ''
}
function pickTarget(r: ReplyT | null) {
  setTarget(r)
  render()
  replyInput.focus()
}

async function toggleLike(q: Question, r?: ReplyT) {
  const t = r ?? q
  t.liked = !t.liked
  t.likes += t.liked ? 1 : -1
  render()
  const ok = await post('/api/like', { id: ID, question_id: q.id, reply_id: r ? r.id : null })
  if (!ok) loadFeed(true)
}

// ---------- delete ----------
let pendingDelete: { q: Question; r?: ReplyT } | null = null

const dsheet = el('div', 'p-sheet')
dsheet.hidden = true
const dcard = el('div', 'p-card')
const dtitle = el('p', 'd-title', 'Delete this post?')
const drow = el('div', 'd-row')
const dcancel = el('button', 'd-cancel', 'Cancel')
dcancel.type = 'button'
const dgo = el('button', 'd-go', 'Delete')
dgo.type = 'button'
drow.append(dcancel, dgo)
dcard.append(dtitle, drow)
dsheet.append(dcard)
document.body.append(dsheet)

function askDelete(q: Question, r?: ReplyT) {
  pendingDelete = { q, r }
  dtitle.textContent = r ? 'Delete this reply?' : 'Delete this post?'
  dsheet.hidden = false
}
function closeDelete() {
  dsheet.hidden = true
  pendingDelete = null
}
dcancel.addEventListener('click', closeDelete)
dsheet.addEventListener('click', (e) => {
  if (e.target === dsheet) closeDelete()
})
dgo.addEventListener('click', async () => {
  const p = pendingDelete
  closeDelete()
  if (!p) return
  const ok = await post('/api/delete', { id: ID, question_id: p.q.id, reply_id: p.r ? p.r.id : null })
  if (!ok) {
    toast('Could not delete. Try again.')
    return
  }
  if (p.r && target === p.r.id) setTarget(null)
  if (!p.r && openId === p.q.id) history.back()
  loadFeed(true)
})

// ---------- polls ----------
async function vote(q: Question, i: number) {
  const p = q.poll
  if (!p || p.mine !== null) return
  p.mine = i
  p.options[i].votes += 1
  p.total += 1
  render()
  await post('/api/vote', { id: ID, question_id: q.id, option: i })
  loadFeed(true)
}

function pollBox(q: Question): HTMLElement | null {
  const p = q.poll
  if (!p) return null
  const box = el('div', 'poll')
  box.addEventListener('click', (e) => e.stopPropagation())
  p.options.forEach((o, i) => {
    if (p.mine === null) {
      const b = el('button', 'po', o.label)
      b.type = 'button'
      b.addEventListener('click', () => vote(q, i))
      box.append(b)
    } else {
      const pct = p.total > 0 ? Math.round((o.votes / p.total) * 100) : 0
      const row = el('div', 'pbar' + (p.mine === i ? ' mine' : ''))
      const fill = el('i')
      fill.style.width = pct + '%'
      row.append(fill, el('span', undefined, o.label), el('span', undefined, pct + '%'))
      box.append(row)
    }
  })
  box.append(el('div', 'poll-meta', `${p.total} ${p.total === 1 ? 'vote' : 'votes'}`))
  return box
}

// ---------- rendering ----------
function renderHome() {
  feedList.replaceChildren()
  if (questions.length === 0) {
    const box = el('div', 'feed-empty')
    box.append(
      el('h2', undefined, 'No questions yet'),
      el('p', undefined, 'Ask about anything on campus and people can answer.'),
    )
    feedList.append(box)
    return
  }
  for (const q of questions) {
    const row = el('article', 'post')
    const main = el('div')
    const acts = el('div', 'acts')
    acts.append(action('heart', q.likes, q.liked, () => toggleLike(q)), action('comment', q.replies.length))
    if (q.mine) acts.append(action('trash', 0, false, () => askDelete(q)))
    main.append(head(q.seconds_ago, q.place, false, q.nick), el('p', 'text', q.text))
    const poll = pollBox(q)
    if (poll) main.append(poll)
    main.append(acts)
    row.append(avatar(q), main)
    row.addEventListener('click', () => openThread(q.id))
    feedList.append(row)
  }
}

function renderThread(q: Question) {
  threadBody.replaceChildren()

  const root = el('article', 'root')
  const top = el('div', 'root-top')
  top.append(avatar(q), head(q.seconds_ago, q.place, false, q.nick))
  const rootActs = el('div', 'acts')
  rootActs.append(
    action('heart', q.likes, q.liked, () => toggleLike(q)),
    action('comment', q.replies.length, false, () => pickTarget(null)),
  )
  if (q.mine) rootActs.append(action('trash', 0, false, () => askDelete(q)))
  root.append(top, el('p', 'text big', q.text))
  const poll = pollBox(q)
  if (poll) root.append(poll)
  root.append(rootActs)
  threadBody.append(root)

  // group replies by the thing they answer
  const kids = new Map<number | null, ReplyT[]>()
  for (const r of q.replies) {
    const list = kids.get(r.parent) ?? []
    list.push(r)
    kids.set(r.parent, list)
  }

  const build = (parent: number | null, depth: number): HTMLElement[] =>
    (kids.get(parent) ?? []).map((r) => {
      const node = el('div', r.id === target ? 'node targeted' : 'node')
      const row = el('div', 'rep')
      const main = el('div')
      const acts = el('div', 'acts')
      if (r.deleted) {
        main.append(el('p', 'text gone', 'deleted'))
      } else {
        acts.append(
          action('heart', r.likes, r.liked, () => toggleLike(q, r)),
          action('comment', (kids.get(r.id) ?? []).length, false, () => pickTarget(r)),
        )
        if (r.mine) acts.append(action('trash', 0, false, () => askDelete(q, r)))
        main.append(head(r.seconds_ago, null, r.was_there, r.nick), el('p', 'text', r.text), acts)
      }
      const children = build(r.id, depth + 1)
      const av = el('div', children.length > 0 && depth < 2 ? 'av line' : 'av')
      av.append(avatar(r.deleted ? undefined : r))
      row.append(av, main)
      node.append(row)
      if (children.length > 0) {
        const box = el('div', depth < 2 ? 'kids' : 'kids flat')
        box.append(...children)
        node.append(box)
      }
      return node
    })

  const nodes = build(null, 0)
  if (nodes.length === 0) {
    threadBody.append(el('p', 'quiet', 'No replies yet. Be the first.'))
  } else {
    const list = el('div', 'replies')
    list.append(...nodes)
    threadBody.append(list)
  }
}

function render() {
  const q = openId === null ? null : (questions.find((x) => x.id === openId) ?? null)
  if (openId !== null && !q) {
    openId = null // expired, or the server restarted
    setTarget(null)
  }
  feedHome.hidden = q !== null
  threadEl.hidden = q === null
  syncChrome()
  const scroller = q ? threadScroll : homeScroll
  const keep = scroller.scrollTop
  if (q) renderThread(q)
  else renderHome()
  scroller.scrollTop = keep
}

async function loadFeed(force = false) {
  if (!force && (document.hidden || !views.feed.classList.contains('active'))) return
  try {
    const r = await fetch(`/api/questions?id=${ID}`)
    if (!r.ok) return
    questions = (await r.json()) as Question[]
    render()
    if (pendingOpen !== null) {
      const id = pendingOpen
      pendingOpen = null
      if (questions.some((x) => x.id === id)) openThread(id)
    }
    // looking at the feed counts as reading the notifications
    if (views.feed.classList.contains('active') && !document.hidden) clearNotifications()
  } catch {
    /* server not reachable, try again next time */
  }
}

function openThread(id: number) {
  openId = id
  setTarget(null)
  history.pushState({ thread: id }, '')
  render()
  threadScroll.scrollTop = 0
}

window.addEventListener('popstate', () => {
  openId = null
  setTarget(null)
  render()
})
byId('thread-back').addEventListener('click', () => history.back())
byId('replying-clear').addEventListener('click', () => pickTarget(null))

// ---------- the composer at the top ----------
let pollOn = false
const optionInputs = () => Array.from(pollFields.querySelectorAll<HTMLInputElement>('.opt-in'))

function setPoll(on: boolean) {
  pollOn = on
  pollFields.hidden = !on
  pollToggle.classList.toggle('on', on)
  pollToggle.setAttribute('aria-pressed', String(on))
  pollToggle.setAttribute('aria-label', on ? 'Remove poll' : 'Add a poll')
  if (on) optionInputs()[0]?.focus()
}

function resetComposer() {
  askText.value = ''
  askText.style.height = ''
  optionInputs().forEach((input, i) => (i < 2 ? (input.value = '') : input.remove()))
  addOption.hidden = false
  setPoll(false)
}

askText.addEventListener('input', () => {
  askText.style.height = 'auto'
  askText.style.height = Math.min(askText.scrollHeight, 220) + 'px'
})
pollToggle.addEventListener('click', () => setPoll(!pollOn))
addOption.addEventListener('click', () => {
  const n = optionInputs().length
  if (n >= 4) return
  const input = el('input', 'opt-in')
  input.maxLength = 80
  input.placeholder = `Option ${n + 1}`
  input.autocomplete = 'off'
  pollFields.insertBefore(input, addOption)
  addOption.hidden = n + 1 >= 4
  input.focus()
})

composer.addEventListener('submit', async (e) => {
  e.preventDefault()
  const text = askText.value.trim()
  if (!text) return
  const body: { id: string; text: string; options?: string[] } = { id: ID, text }
  if (pollOn) {
    const options = optionInputs()
      .map((i) => i.value.trim())
      .filter((v) => v !== '')
    if (options.length < 2) {
      toast('A poll needs at least 2 options.')
      return
    }
    body.options = options
  }
  if (await post('/api/questions', body)) {
    resetComposer()
    loadFeed(true)
  } else {
    toast('Could not post. Try again.')
  }
})

replyForm.addEventListener('submit', async (e) => {
  e.preventDefault()
  const text = replyInput.value.trim()
  if (!text || openId === null) return
  if (await post('/api/reply', { id: ID, question_id: openId, parent_id: target, text })) {
    replyInput.value = ''
    setTarget(null)
    loadFeed(true)
  }
})

setInterval(() => loadFeed(), 10000)

// ---------- notifications ----------
const bell = byId('bell')
const pushOk = 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window
const isIos = /iphone|ipad|ipod/i.test(navigator.userAgent)
const installed =
  window.matchMedia('(display-mode: standalone)').matches ||
  (navigator as Navigator & { standalone?: boolean }).standalone === true

function setBell(on: boolean) {
  bell.classList.toggle('on', on)
  bell.innerHTML = on ? ICONS.bell : ICONS.bellOff
  bell.setAttribute('aria-pressed', String(on))
  bell.setAttribute('aria-label', on ? 'Turn notifications off' : 'Turn notifications on')
}
setBell(false)

async function clearNotifications() {
  if (!('serviceWorker' in navigator)) return
  try {
    const reg = await navigator.serviceWorker.getRegistration()
    const open = (await reg?.getNotifications()) ?? []
    open.forEach((n) => n.close())
  } catch {
    /* not important */
  }
}

async function serverKey(): Promise<Uint8Array<ArrayBuffer>> {
  const r = await fetch('/api/push/key')
  const b64 = (await r.json()) as string
  const raw = atob((b64 + '='.repeat((4 - (b64.length % 4)) % 4)).replace(/-/g, '+').replace(/_/g, '/'))
  return Uint8Array.from(raw, (c) => c.charCodeAt(0))
}

function sameKey(a: ArrayBuffer | null, b: Uint8Array): boolean {
  if (!a) return false
  const x = new Uint8Array(a)
  return x.length === b.length && x.every((v, i) => v === b[i])
}

async function sendSub(sub: PushSubscription): Promise<boolean> {
  const j = sub.toJSON()
  return post('/api/push/subscribe', { id: ID, endpoint: j.endpoint, keys: j.keys })
}

async function turnOn() {
  const perm = await Notification.requestPermission()
  if (perm !== 'granted') {
    toast(perm === 'denied' ? 'Notifications are blocked. Allow them in your browser settings.' : 'Notifications are still off.')
    return
  }
  const reg = await navigator.serviceWorker.ready
  const key = await serverKey()
  const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key })
  if (await sendSub(sub)) {
    setBell(true)
    toast('Notifications are on.')
  } else {
    toast('Could not turn notifications on. Try again.')
  }
}

async function turnOff() {
  const reg = await navigator.serviceWorker.ready
  const sub = await reg.pushManager.getSubscription()
  await sub?.unsubscribe()
  await post('/api/push/unsubscribe', { id: ID })
  setBell(false)
  toast('Notifications are off.')
}

bell.addEventListener('click', async () => {
  if (!pushOk) {
    toast(
      isIos && !installed
        ? 'To get notifications, tap Share, then Add to Home Screen.'
        : 'This browser cannot show notifications.',
    )
    return
  }
  try {
    const reg = await navigator.serviceWorker.ready
    if (await reg.pushManager.getSubscription()) await turnOff()
    else await turnOn()
  } catch {
    toast('Something went wrong with notifications. Try again.')
  }
})

async function startPush() {
  if (!pushOk) return
  try {
    const reg = await navigator.serviceWorker.register('/sw.js')
    await navigator.serviceWorker.ready
    let sub = await reg.pushManager.getSubscription()
    if (!sub || Notification.permission !== 'granted') return
    // if the server was reset it has a new key, so subscribe again with it
    const key = await serverKey()
    if (!sameKey(sub.options.applicationServerKey, key)) {
      await sub.unsubscribe()
      sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key })
    }
    if (await sendSub(sub)) setBell(true)
  } catch {
    /* the bell just stays off */
  }
}
startPush()

// tapping a notification while the site is open
if ('serviceWorker' in navigator) {
  navigator.serviceWorker.addEventListener('message', (e) => {
    const d = e.data as { type?: string; qid?: number | null }
    if (d?.type !== 'open') return
    if (typeof d.qid === 'number') pendingOpen = d.qid
    show('feed')
  })
}

// tapping a notification while the site is closed opens it like this
{
  const params = new URLSearchParams(location.search)
  const q = params.get('q')
  if (q !== null || params.get('feed') !== null) {
    if (q !== null && Number.isFinite(Number(q))) pendingOpen = Number(q)
    history.replaceState(null, '', location.pathname)
    show('feed')
  }
}

// ---------- light / dark ----------
const feedView = byId('feed-view')
const themeBtn = byId('theme')

function applyTheme(light: boolean) {
  feedView.classList.toggle('light', light)
  themeBtn.innerHTML = light ? ICONS.moon : ICONS.sun // the button shows what you will switch to
  try {
    localStorage.setItem('ted-theme', light ? 'light' : 'dark')
  } catch {
    /* private mode, ignore */
  }
}

let startLight = false
try {
  startLight = localStorage.getItem('ted-theme') === 'light'
} catch {
  /* ignore */
}
applyTheme(startLight)
themeBtn.addEventListener('click', () => applyTheme(!feedView.classList.contains('light')))