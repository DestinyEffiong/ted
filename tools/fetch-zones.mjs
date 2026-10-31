// asks OpenStreetMap for named places at Babcock and prints Rust zone lines
const SERVERS = [
  "https://overpass-api.de/api/interpreter",
  "https://overpass.kumi.systems/api/interpreter",
];

async function run(query) {
  for (const url of SERVERS) {
    try {
      const res = await fetch(url, {
        method: "POST",
        body: "data=" + encodeURIComponent(query),
        headers: {
          "Content-Type": "application/x-www-form-urlencoded",
          "User-Agent": "ted-campus-app/0.1",
        },
      });
      const text = await res.text();
      if (!res.ok || text.trim().startsWith("<")) {
        console.error(url, "gave status", res.status, text.slice(0, 200));
        continue;
      }
      return JSON.parse(text).elements;
    } catch (e) {
      console.error(url, "failed:", e.message);
    }
  }
  return [];
}

let elements = await run(`
[out:json][timeout:60];
area["name"="Babcock University"]->.a;
nwr(area.a)["name"];
out center tags;
`);

if (elements.length === 0) {
  console.error("name search found nothing, trying around the campus point");
  elements = await run(`
[out:json][timeout:60];
nwr(around:1500,6.893,3.717)["name"];
out center tags;
`);
}

const seen = new Set();
for (const el of elements) {
  const t = el.tags;
  if (t.highway || t.boundary || t.route) continue;
  const lat = el.lat ?? el.center?.lat;
  const lng = el.lon ?? el.center?.lon;
  if (lat === undefined || seen.has(t.name)) continue;
  seen.add(t.name);
  const name = t.name.replace(/"/g, "");
  console.log(`Zone { name: "${name}", lat: ${lat.toFixed(5)}, lng: ${lng.toFixed(5)}, radius_m: 40.0 },`);
}