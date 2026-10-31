// shows notifications when the server pushes, even when the site is closed

self.addEventListener('install', () => self.skipWaiting())
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()))

self.addEventListener('push', (event) => {
  let d = {}
  try {
    d = event.data ? event.data.json() : {}
  } catch (e) {
    d = {}
  }
  event.waitUntil(show(d))
})

// questions stack into one notification: "<latest question> and 3 others"
// replies stack per thread, so one busy thread does not bury everything else
async function show(d) {
  const text = String(d.text || '')
  const reply = d.kind === 'reply'
  const tag = reply ? 'ted-reply-' + d.qid : 'ted-question'
  const open = await self.registration.getNotifications({ tag })
  const count = open.reduce((n, x) => n + ((x.data && x.data.count) || 1), 0) + 1
  const lead = reply ? '~ replied: ' + text : text
  const body = count > 1 ? lead + ' and ' + (count - 1) + (count === 2 ? ' other' : ' others') : lead
  return self.registration.showNotification('ted', {
    body,
    tag,
    renotify: true,
    data: { qid: d.qid, count, reply },
  })
}

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const qid = event.notification.data && event.notification.data.qid
  // a stacked notification may hold several questions, so only a reply opens its thread directly
  const target = event.notification.data && event.notification.data.reply ? qid : null
  event.waitUntil(
    (async () => {
      const all = await self.clients.matchAll({ type: 'window', includeUncontrolled: true })
      const msg = { type: 'open', qid: target }
      if (all.length > 0) {
        const c = all[0]
        await c.focus()
        c.postMessage(msg)
        return
      }
      await self.clients.openWindow(target === null ? '/?feed=1' : '/?q=' + target)
    })(),
  )
})