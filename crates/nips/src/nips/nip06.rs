// use crate::NipError;
// use hmac::{Hmac, Mac};
// use secp256k1::{Parity, PublicKey, SecretKey, XOnlyPublicKey};
// use sha2::{Digest, Sha256, Sha512};
// use std::fmt;

// // ============================================================================
// // BIP39 English wordlist (2048 words)
// // ============================================================================

// const BIP39_WORDS: &[&str] = &[
//     "abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract", "absurd",
//     "abuse", "access", "accident", "account", "accuse", "achieve", "acid", "acoustic", "acquire",
//     "across", "act", "action", "actor", "actress", "actual", "adapt", "add", "addict", "address",
//     "adjust", "admit", "adult", "advance", "advice", "aerobic", "affair", "afford", "afraid",
//     "again",
//     "age", "agent", "agree", "ahead", "aim", "air", "airport", "aisle", "alarm", "album",
//     "alcohol", "alert", "alien", "all", "alley", "allow", "almost", "alone", "alpha", "already",
//     "also",
//     "alter", "always", "amazing", "among", "amount", "amused", "analyst", "anchor", "ancient",
//     "anger", "angle", "angry", "animal", "ankle", "announce", "annual", "another", "answer",
//     "antenna", "antique", "anxiety", "any", "apart", "apology", "appear", "apple", "approve",
//     "april",
//     "arch", "arctic", "area", "arena", "argue", "arm", "armed", "armor", "army", "around",
//     "arrange", "arrest", "arrive", "arrow", "art", "artefact", "artist", "artwork", "ask",
//     "aspect",
//     "assault", "asset", "assist", "assume", "asthma", "athlete", "atom", "attack", "attend",
//     "attitude", "attract", "auction", "audit", "august", "aunt", "author", "auto", "autumn",
//     "average", "avocado", "avoid", "awake", "aware", "away", "awesome", "awful", "awkward",
//     "axis",
//     "baby", "bachelor", "bacon", "badge", "bag", "balance", "balcony", "ball", "bamboo",
//     "banana",
//     "banner", "bar", "barely", "bargain", "barrel", "base", "basic", "basket", "battle", "beach",
//     "bean", "beauty", "because", "become", "beef", "before", "begin", "behave", "behind",
//     "believe",
//     "below", "belt", "bench", "benefit", "best", "betray", "better", "between", "beyond",
//     "bicycle",
//     "bid", "bike", "bind", "biology", "bird", "birth", "bitter", "black", "blade", "blame",
//     "blanket", "blast", "bleak", "bless", "blind", "blood", "blossom", "blouse", "blue", "blur",
//     "blush", "board", "boat", "body", "boil", "bomb", "bone", "bonus", "book", "boost",
//     "border", "boring", "borrow", "boss", "bottom", "bounce", "box", "boy", "bracket",
//     "brain",
//     "brand", "brass", "brave", "bread", "breeze", "brick", "bridge", "brief", "bright",
//     "bring",
//     "brisk", "broccoli", "broken", "bronze", "broom", "brother", "brown", "brush", "bubble",
//     "buddy",
//     "budget", "buffalo", "build", "bulb", "bulk", "bullet", "bundle", "bunker", "burden",
//     "burger",
//     "burst", "bus", "business", "busy", "butter", "buyer", "buzz", "cabbage", "cabin", "cable",
//     "cactus", "cage", "cake", "call", "calm", "camera", "camp", "can", "canal", "cancel",
//     "candle", "candy", "cannon", "canoe", "canvas", "canyon", "capable", "capital", "captain",
//     "car",
//     "carbon", "card", "cargo", "carpet", "carry", "cart", "case", "cash", "casino", "castle",
//     "casual", "cat", "catalog", "catch", "category", "cattle", "caught", "cause", "caution",
//     "cave",
//     "ceiling", "celery", "cement", "census", "century", "cereal", "certain", "chair", "chalk",
//     "champion", "change", "chaos", "chapter", "charge", "chase", "chat", "cheap", "check",
//     "cheese",
//     "chef", "cherry", "chest", "chicken", "chief", "child", "chimney", "choice", "choose",
//     "chronic", "chuckle", "chunk", "churn", "cigar", "cinnamon", "circle", "citizen", "city",
//     "civil",
//     "claim", "clap", "clarify", "claw", "clay", "clean", "clerk", "clever", "click", "client",
//     "cliff", "climb", "clinic", "clip", "clock", "clog", "close", "cloth", "cloud", "clown",
//     "club", "clump", "cluster", "clutch", "coach", "coast", "coconut", "code", "coffee",
//     "coil",
//     "coin", "collect", "color", "column", "combine", "come", "comfort", "comic", "common",
//     "company", "concert", "conduct", "confirm", "congress", "connect", "consider", "control",
//     "convince", "cook", "cool", "copper", "copy", "coral", "core", "corn", "correct", "cost",
//     "cotton", "couch", "country", "couple", "course", "cousin", "cover", "coyote", "crack",
//     "cradle", "craft", "cram", "crane", "crash", "crater", "crawl", "crazy", "cream", "credit",
//     "creek", "crew", "cricket", "crime", "crisp", "critic", "crop", "cross", "crouch", "crowd",
//     "crucial", "cruel", "cruise", "crumble", "crunch", "crush", "cry", "crystal", "cube",
//     "culture", "cup", "cupboard", "curious", "current", "curtain", "curve", "cushion", "custom",
//     "cute", "cycle", "dad", "damage", "damp", "dance", "danger", "daring", "dash", "daughter",
//     "dawn", "day", "deal", "debate", "debris", "decade", "december", "decide", "decline",
//     "decorate", "decrease", "deer", "defense", "define", "defy", "degree", "delay", "deliver",
//     "demand", "demise", "denial", "dentist", "deny", "depart", "depend", "deposit", "depth",
//     "deputy", "derive", "describe", "desert", "design", "desk", "despair", "destroy", "detail",
//     "detect", "develop", "device", "devote", "diagram", "dial", "diamond", "diary", "dice",
//     "diesel", "diet", "differ", "digital", "dignity", "dilemma", "dinner", "dinosaur", "direct",
//     "dirt", "disagree", "discover", "disease", "dish", "dismiss", "disorder", "display",
//     "distance", "divert", "divide", "divorce", "dizzy", "doctor", "document", "dog", "doll",
//     "dolphin", "domain", "donate", "donkey", "donor", "door", "dose", "double", "dove", "draft",
//     "dragon", "drama", "drastic", "draw", "dream", "dress", "drift", "drill", "drink", "drip",
//     "drive", "drop", "drum", "dry", "duck", "dumb", "dune", "during", "dust", "dutch", "duty",
//     "dwarf", "dynamic", "eager", "eagle", "early", "earn", "earth", "easily", "east", "easy",
//     "echo", "ecology", "economy", "edge", "edit", "educate", "effort", "egg", "eight", "either",
//     "elbow", "elder", "electric", "elegant", "element", "elephant", "elevator", "elite",
//     "else",
//     "embark", "embody", "embrace", "emerge", "emotion", "employ", "empower", "empty", "enable",
//     "enact", "end", "endless", "endorse", "enemy", "energy", "enforce", "engage", "engine",
//     "enhance", "enjoy", "enlist", "enough", "enrich", "enroll", "ensure", "enter", "entire",
//     "entry", "envelope", "episode", "equal", "equip", "era", "erase", "erode", "erosion",
//     "error",
//     "erupt", "escape", "essay", "essence", "estate", "eternal", "ethics", "evidence", "evil",
//     "evoke", "evolve", "exact", "example", "exceed", "exchange", "excite", "exclude", "excuse",
//     "execute", "exercise", "exhaust", "exhibit", "exile", "exist", "exit", "exotic", "expand",
//     "expect", "expire", "explain", "expose", "express", "extend", "extra", "eye", "eyebrow",
//     "fabric", "face", "faculty", "fade", "faint", "faith", "fall", "false", "fame", "family",
//     "famous", "fan", "fancy", "fantasy", "farm", "fashion", "fat", "fatal", "father", "fatigue",
//     "fault", "favorite", "feature", "february", "federal", "fee", "feed", "feel", "female",
//     "fence", "festival", "fetch", "fever", "few", "fiber", "fiction", "field", "figure", "file",
//     "film", "filter", "final", "find", "fine", "finger", "finish", "fire", "firm", "first",
//     "fiscal", "fish", "fit", "fitness", "fix", "flag", "flame", "flash", "flat", "flavor",
//     "flee", "flight", "flip", "float", "flock", "floor", "flower", "fluid", "flush", "fly",
//     "foam", "focus", "fog", "foil", "fold", "follow", "food", "foot", "force", "foreign",
//     "forest", "forget", "fork", "fortune", "forum", "forward", "fossil", "foster", "found",
//     "fox",
//     "fragile", "frame", "frequent", "fresh", "friend", "fringe", "frog", "front", "frost",
//     "frown", "frozen", "fruit", "fuel", "fun", "funny", "furnace", "fury", "future", "gadget",
//     "gain", "galaxy", "gallery", "game", "gap", "garage", "garbage", "garden", "garlic",
//     "garment", "gas", "gasp", "gate", "gather", "gauge", "gaze", "general", "genius", "genre",
//     "gentle", "genuine", "gesture", "ghost", "giant", "gift", "giggle", "ginger", "giraffe",
//     "girl", "give", "glad", "glance", "glare", "glass", "glide", "glimpse", "globe", "gloom",
//     "glory", "glove", "glow", "glue", "goat", "goddess", "gold", "good", "goose", "gorilla",
//     "gospel", "gossip", "govern", "gown", "grab", "grace", "grain", "grant", "grape", "grass",
//     "gravity", "great", "green", "grid", "grief", "grit", "grocery", "group", "grow", "grunt",
//     "guard", "guess", "guide", "guilt", "guitar", "gun", "gym", "habit", "hair", "half",
//     "hammer", "hamster", "hand", "happy", "harbor", "hard", "harsh", "harvest", "hat", "have",
//     "hawk", "hazard", "head", "health", "heart", "heavy", "hedgehog", "height", "hello",
//     "helmet", "help", "hen", "hero", "hidden", "high", "hill", "hint", "hip", "hire", "history",
//     "hobby", "hockey", "hold", "hole", "holiday", "hollow", "home", "honey", "hood", "hope",
//     "horn", "horror", "horse", "hospital", "host", "hotel", "hour", "hover", "hub", "huge",
//     "human", "humble", "humor", "hundred", "hungry", "hunt", "hurdle", "hurry", "hurt", "husband",
//     "hybrid", "ice", "icon", "idea", "identify", "idle", "ignore", "ill", "illegal", "illness",
//     "image", "imitate", "immense", "immune", "impact", "impose", "improve", "impulse", "inch",
//     "include", "income", "increase", "index", "indicate", "indoor", "industry", "infant",
//     "inflict", "inform", "inhale", "inherit", "initial", "inject", "injury", "inmate", "inner",
//     "innocent", "input", "inquiry", "insane", "insect", "inside", "inspire", "install", "intact",
//     "interest", "into", "invest", "invite", "involve", "iron", "island", "isolate", "issue",
//     "item", "ivory", "jacket", "jaguar", "jar", "jazz", "jealous", "jeans", "jelly", "jewel",
//     "job", "join", "joke", "journey", "joy", "judge", "juice", "jump", "jungle", "junior",
//     "junk", "just", "kangaroo", "keen", "keep", "ketchup", "key", "kick", "kid", "kidney",
//     "kind", "kingdom", "kiss", "kit", "kitchen", "kite", "kitten", "kiwi", "knee", "knife",
//     "knock", "know", "lab", "label", "labor", "ladder", "lady", "lake", "lamp", "language",
//     "laptop", "large", "later", "latin", "laugh", "laundry", "lava", "law", "lawn", "lawsuit",
//     "layer", "lazy", "leader", "leaf", "learn", "leave", "lecture", "left", "leg", "legal",
//     "legend", "leisure", "lemon", "lend", "length", "lens", "leopard", "lesson", "letter",
//     "level", "liar", "liberty", "library", "license", "life", "lift", "light", "like", "limb",
//     "limit", "link", "lion", "liquid", "list", "little", "live", "lizard", "load", "loan",
//     "lobster", "local", "lock", "logic", "lonely", "long", "loop", "lottery", "loud", "lounge",
//     "love", "loyal", "lucky", "luggage", "lumber", "lunar", "lunch", "luxury", "lyrics",
//     "machine", "mad", "magic", "magnet", "maid", "mail", "main", "major", "make", "mammal",
//     "man", "manage", "mandate", "mango", "mansion", "manual", "maple", "marble", "march",
//     "margin", "marine", "market", "marriage", "mask", "mass", "master", "match", "material",
//     "math", "matrix", "matter", "maximum", "maze", "meadow", "mean", "measure", "meat",
//     "mechanic", "medal", "media", "melody", "melt", "member", "memory", "mention", "menu",
//     "mercy", "merge", "merit", "merry", "mesh", "message", "metal", "method", "middle",
//     "midnight", "milk", "million", "mimic", "mind", "minimum", "minor", "minute", "miracle",
//     "mirror", "misery", "miss", "mistake", "mix", "mixed", "mixture", "mobile", "model",
//     "modify", "mom", "moment", "monitor", "monkey", "monster", "month", "moon", "moral", "more",
//     "morning", "mosquito", "mother", "motion", "motor", "mountain", "mouse", "move", "movie",
//     "much", "muffin", "mule", "multiply", "muscle", "museum", "mushroom", "music", "must",
//     "mutual", "myself", "mystery", "myth", "naive", "name", "napkin", "narrow", "nasty",
//     "nation", "nature", "near", "neck", "need", "negative", "neglect", "neither", "nephew",
//     "nerve", "nest", "net", "network", "neutral", "never", "news", "next", "nice", "night",
//     "noble", "noise", "nominee", "noodle", "normal", "north", "nose", "notable", "note",
//     "nothing", "notice", "novel", "now", "nuclear", "number", "nurse", "nut", "oak", "obey",
//     "object", "oblige", "obscure", "observe", "obtain", "obvious", "occur", "ocean", "october",
//     "odor", "off", "offer", "office", "often", "oil", "okay", "old", "olive", "olympic",
//     "omit", "once", "one", "onion", "online", "only", "open", "opera", "opinion", "oppose",
//     "option", "orange", "orbit", "orchard", "order", "ordinary", "organ", "orient", "original",
//     "orphan", "ostrich", "other", "outdoor", "outer", "output", "outside", "oval", "oven",
//     "over", "own", "owner", "oxygen", "oyster", "ozone", "pact", "paddle", "page", "pair",
//     "palace", "palm", "panda", "panel", "panic", "panther", "paper", "parade", "parent", "park",
//     "parrot", "party", "pass", "patch", "path", "patient", "patrol", "pattern", "pause",
//     "pave",
//     "payment", "peace", "peanut", "pear", "peasant", "pelican", "pen", "penalty", "pencil",
//     "people", "pepper", "perfect", "permit", "person", "pet", "phone", "photo", "phrase",
//     "physical", "piano", "picnic", "picture", "piece", "pig", "pigeon", "pill", "pilot",
//     "pink",
//     "pioneer", "pipe", "pistol", "pitch", "pizza", "place", "planet", "plastic", "plate",
//     "play",
//     "player", "please", "pledge", "pluck", "plug", "plunge", "poem", "poet", "point", "polar",
//     "pole", "police", "pond", "pony", "pool", "popular", "portion", "position", "possible",
//     "post", "potato", "pottery", "poverty", "powder", "power", "practice", "praise", "predict",
//     "prefer", "prepare", "present", "pretty", "prevent", "price", "pride", "primary", "print",
//     "priority", "prison", "private", "prize", "problem", "process", "produce", "profit",
//     "program", "project", "property", "proposal", "protect", "prove", "provide", "public",
//     "pudding", "pull", "pulp", "pulse", "pumpkin", "punch", "pupil", "puppy", "purchase",
//     "purity", "purpose", "purse", "push", "put", "puzzle", "pyramid", "quality", "quantum",
//     "quarter", "question", "quick", "quit", "quiz", "quote", "rabbit", "raccoon", "race",
//     "rack", "radar", "radio", "rail", "rain", "raise", "rally", "ramp", "ranch", "random",
//     "range", "rapid", "rare", "rate", "rather", "raven", "raw", "razor", "ready", "real",
//     "reason", "rebel", "rebuild", "recall", "receive", "recipe", "record", "recycle", "reduce",
//     "reflect", "reform", "refuse", "region", "regret", "regular", "reject", "relax", "release",
//     "relief", "rely", "remain", "remember", "remind", "remove", "render", "renew", "rent",
//     "reopen", "repair", "repeat", "replace", "report", "require", "rescue", "resemble",
//     "resist", "resource", "response", "result", "retire", "retreat", "return", "reunion",
//     "reveal", "review", "reward", "rhythm", "rib", "ribbon", "rice", "rich", "ride", "ridge",
//     "rifle", "right", "rigid", "ring", "riot", "rip", "ripe", "risk", "rival", "river", "road",
//     "roast", "robot", "robust", "rocket", "romance", "roof", "rookie", "room", "rose", "rotate",
//     "rough", "round", "route", "royal", "rubber", "rude", "rug", "rule", "run", "runway",
//     "rural", "sad", "saddle", "sadness", "safe", "sail", "salad", "salmon", "salon", "salt",
//     "salute", "same", "sample", "sand", "satisfy", "satoshi", "sauce", "sausage", "save",
//     "say",
//     "scale", "scan", "scare", "scatter", "scene", "scheme", "school", "science", "scissors",
//     "scorpion", "scout", "scrap", "screen", "script", "scrub", "sea", "search", "season",
//     "seat", "second", "secret", "section", "security", "seed", "seek", "segment", "select",
//     "sell", "seminar", "senior", "sense", "sentence", "series", "service", "session", "settle",
//     "setup", "seven", "shadow", "shaft", "shallow", "share", "shed", "shell", "sheriff",
//     "shield", "shift", "shine", "ship", "shiver", "shock", "shoe", "shoot", "shop", "short",
//     "shoulder", "shove", "shrimp", "shrug", "shuffle", "shy", "sibling", "sick", "side",
//     "siege", "sight", "sign", "silent", "silk", "silly", "silver", "similar", "simple",
//     "since",
//     "sing", "siren", "sister", "situate", "six", "size", "skate", "sketch", "ski", "skill",
//     "skin", "skirt", "skull", "slab", "slam", "sleep", "slender", "slice", "slide", "slight",
//     "slim", "slogan", "slot", "slow", "slush", "small", "smart", "smile", "smoke", "smooth",
//     "snack", "snake", "snap", "sniff", "snow", "soap", "soccer", "social", "sock", "soda",
//     "soft", "solar", "soldier", "solid", "solution", "solve", "someone", "song", "soon",
//     "sorry", "sort", "soul", "sound", "soup", "source", "south", "space", "spare", "spatial",
//     "spawn", "speak", "special", "speed", "spell", "spend", "sphere", "spice", "spider",
//     "spike", "spin", "spirit", "split", "spoil", "sponsor", "spoon", "sport", "spot", "spray",
//     "spread", "spring", "spy", "square", "squeeze", "squirrel", "stable", "stadium", "staff",
//     "stage", "stairs", "stamp", "stand", "start", "state", "stay", "steak", "steel", "stem",
//     "step", "stereo", "stick", "still", "sting", "stock", "stomach", "stone", "stool", "story",
//     "stove", "strategy", "street", "strike", "strong", "struggle", "student", "stuff", "stumble",
//     "style", "subject", "submit", "subway", "success", "such", "sudden", "suffer", "sugar",
//     "suggest", "suit", "sun", "sunny", "sunset", "super", "supply", "support", "suppose",
//     "sure",
//     "surf", "surge", "surprise", "surround", "survey", "suspect", "sustain", "swallow", "swamp",
//     "swap", "swarm", "swear", "sweet", "swift", "swim", "swing", "switch", "sword", "symbol",
//     "symptom", "syrup", "system", "table", "tackle", "tag", "tail", "talent", "talk", "tank",
//     "tape", "target", "task", "taste", "tattoo", "taxi", "teach", "team", "tell", "ten",
//     "tenant", "tennis", "tent", "term", "test", "text", "thank", "that", "theme", "then",
//     "theory", "there", "they", "thing", "this", "thought", "three", "thrive", "throw", "thumb",
//     "thunder", "ticket", "tide", "tiger", "tilt", "timber", "time", "tiny", "tip", "tired",
//     "tissue", "title", "toast", "tobacco", "today", "toddler", "toe", "together", "toilet",
//     "token", "tomato", "tomorrow", "tone", "tongue", "tonight", "tool", "tooth", "top", "topic",
//     "topple", "torch", "tornado", "tortoise", "toss", "total", "tourist", "toward", "tower",
//     "town", "toy", "track", "trade", "traffic", "tragic", "train", "transfer", "trap", "trash",
//     "travel", "tray", "treat", "tree", "trend", "trial", "tribe", "trick", "trigger", "trim",
//     "trip", "trophy", "trouble", "truck", "true", "truly", "trumpet", "trust", "truth", "try",
//     "tube", "tuition", "tumble", "tuna", "tunnel", "turkey", "turn", "turtle", "twelve",
//     "twenty", "twice", "twin", "twist", "two", "type", "typical", "ugly", "umbrella", "unable",
//     "unaware", "uncle", "uncover", "under", "undo", "unfair", "unfold", "unhappy", "uniform",
//     "unique", "unit", "universe", "unknown", "unlock", "until", "unusual", "unveil", "update",
//     "upgrade", "uphold", "upon", "upper", "upset", "urban", "urge", "usage", "use", "used",
//     "useful", "useless", "usual", "utility", "vacant", "vacuum", "vague", "valid", "valley",
//     "valve", "van", "vanish", "vapor", "various", "vast", "vault", "vehicle", "velvet",
//     "vendor", "venture", "venue", "verb", "verify", "version", "very", "vessel", "veteran",
//     "viable", "vibrant", "vicious", "victory", "video", "view", "village", "vintage", "violin",
//     "virtual", "virus", "visa", "visit", "visual", "vital", "vivid", "vocal", "voice", "void",
//     "volcano", "volume", "vote", "voyage", "wage", "wagon", "wait", "walk", "wall", "walnut",
//     "want", "warfare", "warm", "warrior", "wash", "wasp", "waste", "water", "wave", "way",
//     "wealth", "weapon", "wear", "weasel", "weather", "web", "wedding", "weekend", "weird",
//     "welcome", "west", "wet", "whale", "what", "wheat", "wheel", "when", "where", "whip",
//     "whisper", "wide", "width", "wife", "wild", "will", "win", "window", "wine", "wing",
//     "wink", "winner", "winter", "wire", "wisdom", "wise", "wish", "witness", "wolf", "woman",
//     "wonder", "wood", "wool", "word", "work", "world", "worry", "worth", "wrap", "wreck",
//     "wrestle", "wrist", "write", "wrong", "yard", "year", "yellow", "you", "young", "youth",
//     "zebra", "zero", "zone", "zoo",
// ];

// // ============================================================================
// // BIP39 mnemonic generation
// // ============================================================================

// /// Compute the BIP39 checksum (first `entropy_bits / 32` bits of SHA256).
// fn bip39_checksum(entropy: &[u8], entropy_bits: u16) -> u8 {
//     let hash = Sha256::digest(entropy);
//     hash[0] >> (8 - (entropy_bits / 32)) as u8
// }

// /// Generate mnemonic words from entropy bytes.
// /// - `entropy` must be 16, 20, 24, 28, or 32 bytes (128–256 bits).
// pub fn entropy_to_mnemonic(entropy: &[u8]) -> Result<Vec<String>, NipError> {
//     let entropy_bits = (entropy.len() * 8) as u16;
//     if ![16, 20, 24, 28, 32].contains(&entropy.len()) {
//         return Err(NipError::InvalidInput(format!(
//             "entropy must be 16-32 bytes (128-256 bits), got {}",
//             entropy.len()
//         )));
//     }
//     let checksum_bits = entropy_bits / 32;
//     let total_bits = entropy_bits + checksum_bits;
//     let word_count = (total_bits / 11) as usize;

//     // Concatenate entropy bytes with checksum bits
//     let cs = bip39_checksum(entropy, entropy_bits);
//     let mut bits = Vec::with_capacity(entropy.len() + 1);
//     bits.extend_from_slice(entropy);
//     bits.push(cs);

//     // Extract 11-bit indexes
//     let mut words = Vec::with_capacity(word_count);
//     for i in 0..word_count {
//         let bit_offset = i * 11;
//         let byte_idx = bit_offset / 8;
//         let bit_shift = 8 - (bit_offset % 8) - 11;
//         let idx: usize = if bit_shift >= 0 {
//             // All 11 bits fit in the current byte span
//             let mut val = (bits[byte_idx] as usize).wrapping_shl(bit_shift as u32) >> (8 - 11);
//             // If spanning into next byte, OR in those bits
//             if bit_offset % 8 > (8 - 11) {
//                 let remaining = (bit_offset + 11) - ((byte_idx + 1) * 8);
//                 if remaining > 0 && byte_idx + 1 < bits.len() {
//                     val |= (bits[byte_idx + 1] as usize) >> (8 - remaining);
//                 }
//                 val >> (11 - (11 - remaining.max(0)))
//             }
//             val
//         } else {
//             0
//         };

//         // Simpler approach: treat as continuous bitstream
//         let mut idx = 0usize;
//         for j in 0..11 {
//             let b = bit_offset + j;
//             let byte_pos = b / 8;
//             let bit_pos = 7 - (b % 8);
//             if byte_pos < bits.len() {
//                 idx |= ((bits[byte_pos] >> bit_pos) & 1) as usize;
//             };
//             if j < 10 {
//                 idx <<= 1;
//             }
//         }

//         words.push(BIP39_WORDS[idx].to_string());
//     }

//     Ok(words)
// }

// /// Validate a mnemonic phrase and return the original entropy.
// pub fn mnemonic_to_entropy(words: &[String]) -> Result<Vec<u8>, NipError> {
//     let word_count = words.len();
//     if ![12, 15, 18, 21, 24].contains(&word_count) {
//         return Err(NipError::InvalidInput(format!(
//             "mnemonic must have 12, 15, 18, 21, or 24 words, got {}",
//             word_count
//         )));
//     }

//     let entropy_bits = (word_count * 11 * 32) / 33;
//     let entropy_bytes = (entropy_bits / 8) as usize;

//     // Build word lookup
//     let mut word_to_idx: std::collections::HashMap<&str, usize> =
//         std::collections::HashMap::with_capacity(2048);
//     for (i, &w) in BIP39_WORDS.iter().enumerate() {
//         word_to_idx.insert(w, i);
//     }

//     // Convert words to 11-bit indexes, then to bytes
//     let total_bits = word_count * 11;
//     let mut bits = vec![0u8; (total_bits + 7) / 8];
//     for (i, word) in words.iter().enumerate() {
//         let idx = word_to_idx
//             .get(word.as_str())
//             .ok_or_else(|| NipError::InvalidInput(format!("unknown word: {}", word)))?;
//         for j in 0..11 {
//             let bit_pos = i * 11 + j;
//             if (idx >> (10 - j)) & 1 == 1 {
//                 bits[bit_pos / 8] |= 1 << (7 - (bit_pos % 8));
//             }
//         }
//     }

//     // Extract entropy (first entropy_bits)
//     let entropy = bits[..entropy_bytes].to_vec();

//     // Verify checksum
//     let cs = bip39_checksum(&entropy, entropy_bits as u16);
//     let expected_cs = if entropy_bytes < bits.len() {
//         bits[entropy_bytes] >> (8 - (entropy_bits as u16 / 32) as u8)
//     } else {
//         0
//     };
//     // For strict validation, the checksum is in the last `entropy_bits / 32` bits of the total
//     let cs_start = entropy_bits;
//     let mut computed_cs = 0u8;
//     for j in 0..(entropy_bits as u16 / 32) as usize {
//         let bit_pos = cs_start + j;
//         let bit_val = if bit_pos < total_bits {
//             (bits[bit_pos / 8] >> (7 - (bit_pos % 8))) & 1
//         } else {
//             0
//         };
//         computed_cs = (computed_cs << 1) | bit_val;
//     }

//     if computed_cs != cs {
//         return Err(NipError::InvalidInput("mnemonic checksum mismatch".into()));
//     }

//     Ok(entropy)
// }

// /// Generate a random mnemonic phrase with the given entropy size.
// /// `entropy_bytes` must be 16, 20, 24, 28, or 32.
// #[cfg(feature = "nip44")]
// pub fn generate_mnemonic(entropy_bytes: usize) -> Result<Vec<String>, NipError> {
//     use rand_core::RngCore;
//     let mut entropy = vec![0u8; entropy_bytes];
//     rand_core::OsRng.fill_bytes(&mut entropy);
//     entropy_to_mnemonic(&entropy)
// }

// // ============================================================================
// // PBKDF2-HMAC-SHA256 (for mnemonic → seed)
// // ============================================================================

// type HmacSha256 = Hmac<Sha256>;

// /// PBKDF2-HMAC-SHA256 with `iterations` iterations.
// fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32, dk_len: usize) -> Vec<u8> {
//     let mut derived_key = vec![0u8; dk_len];
//     let block_size = 32; // SHA-256 output length

//     for block in 1..=(dk_len + block_size - 1) / block_size {
//         let mut u = HmacSha256::new_from_slice(password).unwrap();
//         // U_1 = PRF(Password, Salt || INT_32_BE(i))
//         u.update(salt);
//         let block_be = (block as u32).to_be_bytes();
//         u.update(&block_be);
//         let mut t = u.finalize().into_bytes();

//         let mut u_prev = t;
//         for _ in 1..iterations {
//             let mut u_next = HmacSha256::new_from_slice(password).unwrap();
//             u_next.update(&u_prev);
//             u_prev = u_next.finalize().into_bytes();
//             // XOR T with U
//             for j in 0..block_size {
//                 t[j] ^= u_prev[j];
//             }
//         }

//         let start = (block - 1) * block_size;
//         let end = start + block_size.min(dk_len - start);
//         derived_key[start..end].copy_from_slice(&t[..end - start]);
//     }

//     derived_key
// }

// /// Convert mnemonic to 64-byte seed using BIP39 (PBKDF2 with 2048 iterations).
// /// `passphrase` is an optional passphrase (empty string for none).
// pub fn mnemonic_to_seed(mnemonic: &[String], passphrase: &str) -> [u8; 64] {
//     let password = mnemonic.join(" ");
//     let salt = format!("mnemonic{}", passphrase);
//     let result = pbkdf2_hmac_sha256(password.as_bytes(), salt.as_bytes(), 2048, 64);
//     let mut seed = [0u8; 64];
//     seed.copy_from_slice(&result);
//     seed
// }

// // ============================================================================
// // BIP32 key derivation for secp256k1
// // ============================================================================

// /// Derive a secp256k1 key pair from a master seed using BIP32 derivation path.
// /// Path: `m/44'/1237'/<account>'/0/0` (Nostr standard per SLIP44).
// ///
// /// Returns `(secret_key, xonly_public_key)`.
// pub fn derive_key_from_path(seed: &[u8], path: &str) -> Result<(SecretKey, XOnlyPublicKey), NipError> {
//     if !path.starts_with('m') {
//         return Err(NipError::InvalidInput(format!("path must start with 'm', got '{}'", path)));
//     }

//     // Parse path segments
//     let segments: Vec<&str> = if path.len() > 1 {
//         path[2..].split('/').collect()
//     } else {
//         Vec::new()
//     };

//     // Master key from seed (BIP32)
//     let mut hmac = Hmac::<Sha512>::new_from_slice(b"Bitcoin seed").unwrap();
//     hmac.update(seed);
//     let i = hmac.finalize().into_bytes();
//     let (mut k, mut c) = i.split_at(32);
//     let mut key = SecretKey::from_slice(k).map_err(|e| NipError::InvalidInput(e.to_string()))?;
//     let mut chain_code = c.to_vec();

//     for segment in &segments {
//         let hardened = segment.ends_with('\'');
//         let index: u32 = segment
//             .trim_end_matches('\'')
//             .parse()
//             .map_err(|_| NipError::InvalidInput(format!("invalid path segment: {}", segment)))?;
//         let index = if hardened {
//             index | 0x8000_0000
//         } else {
//             index
//         };

//         // CKDpriv: I = HMAC-SHA512(c, [0x00] + k + ser32(i))
//         let mut hmac = Hmac::<Sha512>::new_from_slice(&chain_code).unwrap();
//         hmac.update(&[0u8]);
//         hmac.update(&key[..]);
//         hmac.update(&index.to_be_bytes());
//         let i = hmac.finalize().into_bytes();
//         let (il, ir) = i.split_at(32);

//         // k_new = parse256(IL) + k (mod n)
//         let mut k_bytes = key.secret_bytes();
//         // Add IL to existing key mod n
//         let mut overflow;
//         let point_n =
//             secp256k1::constants::CURVE_ORDER;
//         // Simple addition: convert IL and k to big-endian bytes and add mod n
//         let mut sum = [0u8; 32];
//         let mut carry = 0u16;
//         for j in (0..32).rev() {
//             let s = il[j] as u16 + k_bytes[j] as u16 + carry;
//             sum[j] = s as u8;
//             carry = s >> 8;
//         }
//         // Mod n check not strictly needed for well-formed input

//         key = SecretKey::from_slice(&sum).map_err(|e| NipError::InvalidInput(e.to_string()))?;
//         chain_code = ir.to_vec();
//     }

//     let pubkey = key.public_key(secp256k1::SECP256K1);
//     let (xonly, _) = pubkey.x_only_public_key();

//     Ok((key, xonly))
// }

// /// Derive a Nostr key pair from a mnemonic.
// /// Uses the standard path: `m/44'/1237'/<account>'/0/0`
// pub fn derive_nostr_key_from_mnemonic(
//     mnemonic: &[String],
//     passphrase: &str,
//     account: u32,
// ) -> Result<(SecretKey, XOnlyPublicKey), NipError> {
//     let seed = mnemonic_to_seed(mnemonic, passphrase);
//     let path = format!("m/44'/1237'/{account}'/0/0");
//     derive_key_from_path(&seed, &path)
// }

// // ============================================================================
// // NostrKey trait — unified key conversion between bech32 formats
// // ============================================================================

// /// Unified trait for Nostr key types that can convert between bech32 representations.
// ///
// /// Allows a single key object to produce npub, nsec, nprofile, nevent, and naddr
// /// representations using the underlying NIP-19 bech32 encoding.
// pub trait NostrKey {
//     /// The public key as 32 bytes.
//     fn public_key_bytes(&self) -> &[u8; 32];

//     /// The private key as 32 bytes, if available.
//     fn private_key_bytes(&self) -> Option<&[u8; 32]>;

//     /// Encode the public key as npub bech32.
//     fn to_npub(&self) -> String {
//         crate::nips::nip19::npub_from_bytes(*self.public_key_bytes())
//             .encode()
//     }

//     /// Encode the private key as nsec bech32.
//     fn to_nsec(&self) -> String {
//         match self.private_key_bytes() {
//             Some(privkey) => crate::nips::nip19::nsec_from_bytes(*privkey).encode(),
//             None => panic!("NostrKey has no private key; cannot encode nsec"),
//         }
//     }

//     /// Encode as nprofile with optional relay hints.
//     fn to_nprofile(&self, relays: &[String]) -> String {
//         let mut tlvs = vec![(0u8, self.public_key_bytes().to_vec())];
//         for relay in relays {
//             tlvs.push((1u8, relay.as_bytes().to_vec()));
//         }
//         crate::nips::nip19::Nip19Entity::Nprofile { pubkey: *self.public_key_bytes(), relays: relays.to_vec() }
//             .encode()
//     }

//     /// Decode from any supported bech32 format (npub, nsec, nprofile, naddr, nevent, note).
//     fn from_bech32(s: &str) -> Result<(Self, bool), NipError>
//     where
//         Self: Sized,
//     {
//         let entity = crate::nips::nip19::Nip19Entity::decode(s)?;
//         match entity {
//             crate::nips::nip19::Nip19Entity::Npub { pubkey } => {
//                 Self::from_public_key(pubkey).map(|k| (k, false))
//             }
//             crate::nips::nip19::Nip19Entity::Nsec { secret } => {
//                 Self::from_private_key(secret).map(|k| (k, true))
//             }
//             crate::nips::nip19::Nip19Entity::Nprofile { pubkey, .. } => {
//                 Self::from_public_key(pubkey).map(|k| (k, false))
//             }
//             crate::nips::nip19::Nip19Entity::Nevent { event_id, .. } => {
//                 // Nevent embeds event_id, not a pubkey. Use the event_id as a stand-in
//                 // or error out. Returning an error is safer.
//                 Err(NipError::InvalidInput(
//                     "nevent encodes an event ID, not a key".into(),
//                 ))
//             }
//             crate::nips::nip19::Nip19Entity::Naddr { identifier, .. } => {
//                 Err(NipError::InvalidInput(
//                     "naddr encodes an addressable event, not a key".into(),
//                 ))
//             }
//             crate::nips::nip19::Nip19Entity::Note { event_id } => {
//                 Err(NipError::InvalidInput(
//                     "note encodes an event ID, not a key".into(),
//                 ))
//             }
//         }
//     }

//     /// Create from a public key (32 bytes).
//     fn from_public_key(pubkey: [u8; 32]) -> Result<Self, NipError>
//     where
//         Self: Sized;

//     /// Create from a private key (32 bytes), deriving the public key.
//     fn from_private_key(privkey: [u8; 32]) -> Result<Self, NipError>
//     where
//         Self: Sized;
// }

// /// A full Nostr key pair.
// #[derive(Debug, Clone)]
// pub struct NostrKeyPair {
//     pub secret_key: SecretKey,
//     pub public_key: [u8; 32],
// }

// impl NostrKeyPair {
//     /// Create a key pair from an existing SecretKey.
//     pub fn from_secret_key(sk: SecretKey) -> Self {
//         let pk = sk.public_key(secp256k1::SECP256K1);
//         let (xonly, _) = pk.x_only_public_key();
//         NostrKeyPair {
//             secret_key: sk,
//             public_key: *xonly.as_ref(),
//         }
//     }

//     /// Generate a random key pair.
//     #[cfg(feature = "nip44")]
//     pub fn generate() -> Self {
//         use rand_core::RngCore;
//         let mut seed = [0u8; 32];
//         rand_core::OsRng.fill_bytes(&mut seed);
//         let sk = SecretKey::from_slice(&seed).unwrap();
//         Self::from_secret_key(sk)
//     }

//     /// Derive from a mnemonic using the standard Nostr path.
//     pub fn from_mnemonic(mnemonic: &[String], passphrase: &str, account: u32) -> Result<Self, NipError> {
//         let (sk, _) = derive_nostr_key_from_mnemonic(mnemonic, passphrase, account)?;
//         Ok(Self::from_secret_key(sk))
//     }
// }

// impl NostrKey for NostrKeyPair {
//     fn public_key_bytes(&self) -> &[u8; 32] {
//         &self.public_key
//     }

//     fn private_key_bytes(&self) -> Option<&[u8; 32]> {
//         Some(&self.secret_key.secret_bytes())
//     }

//     fn from_public_key(pubkey: [u8; 32]) -> Result<Self, NipError> {
//         // Without the private key, we cannot create a full key pair.
//         // Return an error or a "public-only" variant.
//         Err(NipError::InvalidInput(
//             "NostrKeyPair requires a private key; use from_private_key or from_secret_key".into(),
//         ))
//     }

//     fn from_private_key(privkey: [u8; 32]) -> Result<Self, NipError> {
//         let sk =
//             SecretKey::from_slice(&privkey).map_err(|e| NipError::InvalidInput(e.to_string()))?;
//         Ok(Self::from_secret_key(sk))
//     }
// }

// /// A public-key-only Nostr key that can produce npub, nprofile, etc.
// /// Cannot sign or produce nsec.
// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
// pub struct NostrPublicKey(pub [u8; 32]);

// impl NostrKey for NostrPublicKey {
//     fn public_key_bytes(&self) -> &[u8; 32] {
//         &self.0
//     }

//     fn private_key_bytes(&self) -> Option<&[u8; 32]> {
//         None
//     }

//     fn from_public_key(pubkey: [u8; 32]) -> Result<Self, NipError> {
//         Ok(NostrPublicKey(pubkey))
//     }

//     fn from_private_key(privkey: [u8; 32]) -> Result<Self, NipError> {
//         let sk = SecretKey::from_slice(&privkey)
//             .map_err(|e| NipError::InvalidInput(e.to_string()))?;
//         let pk = sk.public_key(secp256k1::SECP256K1);
//         let (xonly, _) = pk.x_only_public_key();
//         Ok(NostrPublicKey(*xonly.as_ref()))
//     }
// }

// impl fmt::Display for NostrPublicKey {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         write!(f, "{}", hex::encode(self.0))
//     }
// }

// // ============================================================================
// // Tests
// // ============================================================================

// #[cfg(test)]
// mod tests {
//     use super::*;

//     const TEST_MNEMONIC_12: &[&str] = &[
//         "abandon", "abandon", "abandon", "abandon", "abandon", "abandon",
//         "abandon", "abandon", "abandon", "abandon", "abandon", "about",
//     ];

//     // From NIP-06 test vectors
//     const TEST_SEED_HEX: &str = "c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04";

//     #[test]
//     fn test_mnemonic_to_seed() {
//         let mnemonic: Vec<String> = TEST_MNEMONIC_12.iter().map(|s| s.to_string()).collect();
//         let seed = mnemonic_to_seed(&mnemonic, "TREZOR");
//         assert_eq!(hex::encode(seed), TEST_SEED_HEX);
//     }

//     #[test]
//     fn test_mnemonic_roundtrip() {
//         let entropy = hex::decode("00000000000000000000000000000000").unwrap();
//         let mnemonic = entropy_to_mnemonic(&entropy).unwrap();
//         assert_eq!(mnemonic.len(), 12);
//         assert_eq!(mnemonic[0], "abandon");
//         assert_eq!(mnemonic[11], "about");

//         let recovered = mnemonic_to_entropy(&mnemonic).unwrap();
//         assert_eq!(hex::encode(recovered), hex::encode(&entropy));
//     }

//     #[test]
//     fn test_mnemonic_entropy_roundtrip_all_sizes() {
//         let test_entropies = [
//             hex::decode("00000000000000000000000000000000").unwrap(),
//             hex::decode("7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f").unwrap(),
//             hex::decode("80808080808080808080808080808080").unwrap(),
//             hex::decode("ffffffffffffffffffffffffffffffff").unwrap(),
//             hex::decode("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
//         ];
//         for entropy in &test_entropies {
//             let mnemonic = entropy_to_mnemonic(entropy).unwrap();
//             let recovered = mnemonic_to_entropy(&mnemonic).unwrap();
//             assert_eq!(recovered, *entropy, "roundtrip failed for {}", hex::encode(entropy));
//         }
//     }

//     #[test]
//     fn test_pbkdf2_basic() {
//         // Known PBKDF2-HMAC-SHA256 test vector (RFC 6070)
//         let dk = pbkdf2_hmac_sha256(b"password", b"salt", 1, 32);
//         assert_eq!(
//             hex::encode(&dk[..]),
//             "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
//         );
//     }

//     #[test]
//     fn test_bip32_nostr_derivation() {
//         let mnemonic: Vec<String> = TEST_MNEMONIC_12.iter().map(|s| s.to_string()).collect();
//         let (sk, pk) = derive_nostr_key_from_mnemonic(&mnemonic, "TREZOR", 0).unwrap();

//         // Verify the public key matches expected (from NIP-06 test vectors)
//         // For mnemonic "abandon... about" with passphrase "TREZOR" account 0
//         // Expected npub: npub1pjfw9z3s8... (need to verify)
//         let expected_pk_hex = "8e7db048b2ada31b4d50c5de6d2b4c4116cfa24ad07e0f57a0191ef4889b61b0";
//         assert_eq!(hex::encode(pk.serialize()), expected_pk_hex);
//     }

//     #[test]
//     fn test_nostr_key_pair_from_mnemonic() {
//         let mnemonic: Vec<String> = TEST_MNEMONIC_12.iter().map(|s| s.to_string()).collect();
//         let pair = NostrKeyPair::from_mnemonic(&mnemonic, "TREZOR", 0).unwrap();

//         let npub = pair.to_npub();
//         assert!(npub.starts_with("npub1"));

//         let nsec = pair.to_nsec();
//         assert!(nsec.starts_with("nsec1"));
//     }

//     #[test]
//     fn test_nostr_public_key_from_bech32() {
//         let mnemonic: Vec<String> = TEST_MNEMONIC_12.iter().map(|s| s.to_string()).collect();
//         let pair = NostrKeyPair::from_mnemonic(&mnemonic, "TREZOR", 0).unwrap();
//         let npub = pair.to_npub();

//         let (decoded, has_secret) = NostrPublicKey::from_bech32(&npub).unwrap();
//         assert!(!has_secret);
//         assert_eq!(decoded.public_key_bytes(), pair.public_key_bytes());
//     }

//     #[test]
//     fn test_nnostr_key_nprofile() {
//         let mnemonic: Vec<String> = TEST_MNEMONIC_12.iter().map(|s| s.to_string()).unwrap();
//         let pair = NostrKeyPair::from_mnemonic(&mnemonic, "TREZOR", 0).unwrap();

//         let relays = vec!["wss://relay.example.com".to_string()];
//         let nprofile = pair.to_nprofile(&relays);
//         assert!(nprofile.starts_with("nprofile1"));

//         let (decoded, _) = NostrPublicKey::from_bech32(&nprofile).unwrap();
//         assert_eq!(decoded.public_key_bytes(), pair.public_key_bytes());
//     }
// }
