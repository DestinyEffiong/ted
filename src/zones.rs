use serde::Serialize;

#[derive(Serialize)]
pub struct Zone {
    pub name: &'static str,
    pub lat: f64,
    pub lng: f64,
    pub radius_m: f64,
}

pub const ZONES: &[Zone] = &[
    Zone { name: "BU Mall", lat: 6.89123, lng: 3.72044, radius_m: 40.0 },
    Zone { name: "Babcock Guest House", lat: 6.89051, lng: 3.71992, radius_m: 40.0 },
    Zone { name: "New Horizon", lat: 6.89034, lng: 3.72318, radius_m: 40.0 },
    Zone { name: "Neal Wilson Hall", lat: 6.89305, lng: 3.72172, radius_m: 40.0 },
    Zone { name: "School Cafeteria", lat: 6.89268, lng: 3.72361, radius_m: 40.0 },
    Zone { name: "Amphitheatre", lat: 6.89088, lng: 3.72242, radius_m: 40.0 },
    Zone { name: "Winslow Hall", lat: 6.89401, lng: 3.72166, radius_m: 40.0 },
    Zone { name: "Bursary", lat: 6.89010, lng: 3.72289, radius_m: 40.0 },
    Zone { name: "Gideon Troopers Hall", lat: 6.89442, lng: 3.72249, radius_m: 40.0 },
    Zone { name: "Bethel Splendor Hall", lat: 6.89461, lng: 3.72307, radius_m: 40.0 },
    Zone { name: "Samuel Akande Hall", lat: 6.89413, lng: 3.72364, radius_m: 40.0 },
    Zone { name: "Nelson Mandela Hall", lat: 6.89347, lng: 3.72304, radius_m: 40.0 },
    Zone { name: "Queen Esther", lat: 6.89298, lng: 3.72470, radius_m: 40.0 },
    Zone { name: "Felicia Adebisi Dada", lat: 6.89367, lng: 3.72498, radius_m: 40.0 },
    Zone { name: "BUSA House", lat: 6.89205, lng: 3.72376, radius_m: 40.0 },
    Zone { name: "Nyberg Hall", lat: 6.89260, lng: 3.72538, radius_m: 40.0 },
    Zone { name: "Crystal Hall", lat: 6.89286, lng: 3.72774, radius_m: 40.0 },
    Zone { name: "Platinum Hall", lat: 6.89245, lng: 3.72742, radius_m: 40.0 },
    Zone { name: "Diamond Hall", lat: 6.89212, lng: 3.72719, radius_m: 40.0 },
    Zone { name: "Ogden Hall", lat: 6.89283, lng: 3.72638, radius_m: 40.0 },
    Zone { name: "BBS Babcock Business School", lat: 6.89081, lng: 3.72391, radius_m: 40.0 },
    Zone { name: "Science and Technology", lat: 6.88872, lng: 3.72255, radius_m: 40.0 },
    Zone { name: "Senate Building", lat: 6.88884, lng: 3.72188, radius_m: 40.0 },
    Zone { name: "Main Gate", lat: 6.88908, lng: 3.72004, radius_m: 40.0 },
    Zone { name: "White Hall", lat: 6.89377, lng: 3.72633, radius_m: 40.0 },
    Zone { name: "Stadium Babcock University", lat: 6.89472, lng: 3.72770, radius_m: 40.0 },
    Zone { name: "Seventh Day Adventist Church", lat: 6.88961, lng: 3.71960, radius_m: 40.0 },
    Zone { name: "Banks Cluster", lat: 6.89080, lng: 3.72074, radius_m: 40.0 },
    Zone { name: "Welch Hall", lat: 6.89175, lng: 3.72144, radius_m: 40.0 },
    Zone { name: "Laz Otti Library", lat: 6.89216, lng: 3.72233, radius_m: 40.0 },
    Zone { name: "Adeleke Hall", lat: 6.89280, lng: 3.72115, radius_m: 40.0 },
    Zone { name: "Topaz Hall", lat: 6.89346, lng: 3.72061, radius_m: 40.0 },
    Zone { name: "Ameyo Adadevoh Hall", lat: 6.89494, lng: 3.72493, radius_m: 40.0 },
    Zone { name: "Havilah Gold Hall", lat: 6.89488, lng: 3.72607, radius_m: 40.0 },
    Zone { name: "Basketball Court BU", lat: 6.89393, lng: 3.72841, radius_m: 40.0 },
    Zone { name: "BU Highschool", lat: 6.89011, lng: 3.71897, radius_m: 40.0 },
    Zone { name: "Emerald Hall", lat: 6.89370, lng: 3.71999, radius_m: 40.0 },
    // Zone { name: "Babcock University", lat: 6.89000, lng: 3.72297, radius_m: 40.0 },
    // Zone { name: "BUSA", lat: 6.89212, lng: 3.72400, radius_m: 40.0 },
    Zone { name: "BUTH", lat: 6.89181, lng: 3.71850, radius_m: 40.0 },
    Zone { name: "BuCodel", lat: 6.89165, lng: 3.72336, radius_m: 40.0 },
    // Zone { name: "BUTH 600", lat: 6.89126, lng: 3.71694, radius_m: 40.0 },
    Zone { name: "Academic Building", lat: 6.89144, lng: 3.72298, radius_m: 40.0 },
    Zone { name: "EAH", lat: 6.89023, lng: 3.72057, radius_m: 40.0 },
    Zone { name: "Topaz Activity Hall", lat: 6.89376, lng: 3.72028, radius_m: 40.0 },
    Zone { name: "Andrews Park", lat: 6.88738, lng: 3.72135, radius_m: 40.0 },
    Zone { name: "New Bucodel Building", lat: 6.89022, lng: 3.72352, radius_m: 40.0 },
    Zone { name: "New Horizon gazebo 1", lat: 6.89009, lng: 3.72332, radius_m: 40.0 },
    Zone { name: "New Horizon Gezebo 2", lat: 6.89001, lng: 3.72340, radius_m: 40.0 },
    Zone { name: "New Horizon Gazebo 3", lat: 6.88993, lng: 3.72348, radius_m: 40.0 },
    Zone { name: "Babcock University Central Store", lat: 6.88996, lng: 3.72438, radius_m: 40.0 },
    Zone { name: "Facilities Building", lat: 6.88967, lng: 3.72405, radius_m: 40.0 },
    Zone { name: "CIBN building", lat: 6.89013, lng: 3.72390, radius_m: 40.0 },
];

// straight-line distance in metres between two points on earth
fn distance_m(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    let r = 6_371_000.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlng = (lng2 - lng1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlng / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

// the nearest zone this point is inside, if any
pub fn find_zone(lat: f64, lng: f64) -> Option<&'static str> {
    let mut best: Option<(&'static str, f64)> = None;
    for z in ZONES.iter() {
        let d = distance_m(lat, lng, z.lat, z.lng);
        if d <= z.radius_m && best.map_or(true, |(_, bd)| d < bd) {
            best = Some((z.name, d));
        }
    }
    best.map(|(name, _)| name)
}

// nicknames people use, mapped to the real zone name. grow this list as you hear new ones.
const ALIASES: &[(&str, &str)] = &[
    ("caf", "School Cafeteria"),
    ("cafeteria", "School Cafeteria"),
    ("library", "Laz Otti Library"),
    ("mall", "BU Mall"),
    ("gate", "Main Gate"),
    ("senate", "Senate Building"),
    ("stadium", "Stadium Babcock University"),
    ("bucodel", "BuCodel"),
];

// lowercase, strip punctuation, single spaces
fn norm(s: &str) -> String {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// words too general to point at one place
const COMMON: &[&str] = &[
    "hall", "building", "new", "babcock", "university", "school", "house", "park", "science",
    "technology", "business", "main", "activity", "central", "store", "gazebo", "gezebo",
    "cluster", "court", "church", "high", "guest", "basketball", "academic",
];

// does this text mention a campus place? matches whole words only
pub fn place_in_text(text: &str) -> Option<&'static str> {
    let t = format!(" {} ", norm(text));
    // 1. a full zone name
    for z in ZONES.iter() {
        if t.contains(&format!(" {} ", norm(z.name))) {
            return Some(z.name);
        }
    }
    // 2. a nickname
    for (alias, zone) in ALIASES {
        if t.contains(&format!(" {} ", alias)) {
            return Some(*zone);
        }
    }
    // 3. one distinctive word from a name, like "mandela" or "bursary"
    for z in ZONES.iter() {
        for w in norm(z.name).split(' ') {
            if w.len() < 4 || COMMON.contains(&w) {
                continue;
            }
            let owners = ZONES.iter().filter(|o| norm(o.name).split(' ').any(|x| x == w)).count();
            if owners == 1 && t.contains(&format!(" {} ", w)) {
                return Some(z.name);
            }
        }
    }
    None
}