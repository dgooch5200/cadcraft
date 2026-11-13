use super::*;
use cadcraft_doc::EntityKind;
use std::collections::HashSet;

fn prod(ports: &[(Protocol, u32, u32)]) -> Product {
    let mut counts = [(0, 0); 10];
    for (p, i, o) in ports {
        counts[p.index()] = (*i, *o);
    }
    Product { id: "X".into(), name: "X".into(), code: "X".into(), counts, ..Product::default() }
}

#[test]
fn accessories_fit_the_fixture_family() {
    let p = |id: &str, cat: &str| Product { id: id.into(), name: id.into(), cat: cat.into(), ..Product::default() };
    let lib = [
        p("par", "Ovation — PAR"),
        p("rack", "GeNetix"),
        p("general", "General Accessory"),
        p("ovation", "Ovation Series Accessories"),
        p("genetix", "GeNetix Accessories"),
    ];
    let ids = |f: &[&Product]| accessory_products(&lib, f).iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&[&lib[0]]), ["general", "ovation"]);
    assert_eq!(ids(&[&lib[0], &lib[1]]), ["general", "ovation", "genetix"]);
    assert_eq!(ids(&[]), ["general", "ovation", "genetix"]);
    assert_eq!(accessory_products(&lib[..2], &[]).len(), 2);
    assert!(is_accessory(&p("b", "Barn Door")) && !is_accessory(&p("c", "Wash")));
}

fn link(a: u64, b: u64, protocol: Protocol) -> WireLink {
    WireLink::new(Handle(a), Side::Right, Handle(b), Side::Left, protocol)
}

fn code(devs: &[(u64, &Product)], links: &[WireLink], l: &WireLink) -> Option<&'static str> {
    let devs: Vec<_> = devs.iter().map(|(h, p)| (Handle(*h), *p)).collect();
    can_connect(&devs, links, l).err().map(|r| r.code)
}

#[test]
fn can_connect_basics() {
    let none = prod(&[]);
    let dmx = prod(&[(Protocol::Dmx, 1, 1)]);
    let devs = [(1, &dmx), (2, &dmx), (3, &dmx), (4, &none)];
    assert_eq!(code(&devs, &[], &link(1, 2, Protocol::Dmx)), None);
    assert_eq!(code(&devs, &[], &link(1, 1, Protocol::Dmx)), Some("self-connection"));
    assert_eq!(code(&devs, &[], &link(1, 9, Protocol::Dmx)), Some("unknown-device"));

    let l = devs.iter().map(|(h, p)| (Handle(*h), *p)).collect::<Vec<_>>();
    let r = can_connect(&l, &[], &link(1, 4, Protocol::Dmx)).unwrap_err();
    assert_eq!((r.code, r.message.as_str()), ("no-ports-of-type", "X has no DMX ports — check the active wire type"));

    // In and out are one budget: device 1 has 2 DMX ports, both used.
    let used = [link(1, 2, Protocol::Dmx), link(1, 3, Protocol::Dmx)];
    let r = can_connect(&l, &used, &link(1, 2, Protocol::Dmx)).unwrap_err();
    assert_eq!((r.code, r.message.as_str()), ("ports-exhausted", "X: all DMX connections used (max 2)"));
}

#[test]
fn rj45_pool_is_asymmetric() {
    let poe = prod(&[(Protocol::Network, 0, 2)]);
    let data = prod(&[(Protocol::NetData, 1, 0)]);
    let drive = prod(&[(Protocol::Network, 0, 2), (Protocol::NetData, 0, 1)]);
    let sink = prod(&[(Protocol::Network, 8, 0), (Protocol::NetData, 8, 0)]);

    // PoE into a data-only device has no port; data into a PoE-only device is fine.
    assert_eq!(code(&[(1, &poe), (2, &data)], &[], &link(1, 2, Protocol::Network)), Some("no-ports-of-type"));
    assert_eq!(code(&[(1, &poe), (2, &poe)], &[], &link(1, 2, Protocol::NetData)), None);

    // PoE runs count against PoE ports alone, and the refusal says so.
    let devs = [(1, &drive), (2, &sink), (3, &sink)];
    let two_poe = [link(1, 2, Protocol::Network), link(1, 3, Protocol::Network)];
    let l: Vec<_> = devs.iter().map(|(h, p)| (Handle(*h), *p)).collect();
    let r = can_connect(&l, &two_poe, &link(1, 2, Protocol::Network)).unwrap_err();
    assert_eq!((r.code, r.message.as_str()), ("ports-exhausted", "X: all PoE connections used (max 2)"));

    // A data cable may use the spare RJ45, but PoE runs occupy RJ45s too.
    assert_eq!(code(&devs, &two_poe, &link(1, 2, Protocol::NetData)), None);
    let full = [link(1, 2, Protocol::Network), link(1, 3, Protocol::Network), link(1, 3, Protocol::NetData)];
    let r = can_connect(&l, &full, &link(1, 2, Protocol::NetData)).unwrap_err();
    assert_eq!(r.message, "X: all Ethernet ports in use (3 total)");
}

#[test]
fn xdata_round_trip_and_hostile() {
    let w = WireLink::new(Handle(0xAB), Side::Top, Handle(0x1F), Side::Bottom, Protocol::Scene10);
    let x = w.to_xdata();
    assert_eq!(WireLink::from_xdata(&x), Some(w.clone()));

    let other = |app: &str| [RawTag { code: 1001, value: app.into() }, RawTag { code: 1000, value: "zz".into() }];
    let mut mixed: Vec<RawTag> = other("A").to_vec();
    mixed.extend(x.clone());
    mixed.extend(other("B"));
    assert_eq!(WireLink::from_xdata(&mixed), Some(w));

    assert_eq!(WireLink::from_xdata(&[]), None);
    assert_eq!(WireLink::from_xdata(&x[..4]), None);
    assert_eq!(WireLink::from_xdata(&other("CSD")), None);
    let mut wrong_app = x.clone();
    wrong_app[0].value = "OTHER".into();
    assert_eq!(WireLink::from_xdata(&wrong_app), None);
    for bad in ["", "xyz", "FFFFFFFFFFFFFFFFFF", "-1"] {
        let mut b = x.clone();
        b[1].value = bad.into();
        assert_eq!(WireLink::from_xdata(&b), None, "{bad}");
    }
}

const CARD: Vec2 = Vec2 { x: CARD_W, y: CARD_H };

#[test]
fn default_product_card_is_the_fixed_size_and_its_ports_did_not_move() {
    let p = sample_products()[0].clone();
    assert!(p.extra.get("iconW").is_none());
    assert_eq!(card_size(&p, false), CARD);
    let at = Vec2 { x: 10.0, y: 20.0 };
    let one = Vec2 { x: 1.0, y: 1.0 };
    let port = |side| port_point(at, 0.0, one, card_size(&p, false), side, Slot::SOLO);
    assert!(port(Side::Bottom).near(Vec2 { x: 10.0, y: 20.0 - 36.5 }, 1e-9));
    assert!(port(Side::Right).near(Vec2 { x: 45.0, y: 20.0 }, 1e-9));
}

fn wide() -> Product {
    let mut p = sample_products()[0].clone();
    p.extra.insert("iconW".into(), serde_json::json!(216));
    p.extra.insert("iconH".into(), serde_json::json!("72"));
    p
}

#[test]
fn card_size_follows_the_product_and_grows_a_strip_for_two_line() {
    let p = wide();
    assert_eq!(card_size(&p, false), Vec2 { x: 216.0, y: 90.0 });
    assert_eq!(card_size(&p, true), Vec2 { x: 216.0, y: 108.0 });
    assert_eq!(card_size(&builtin_products()[0], true), Vec2 { x: 216.0, y: 72.0 });
    assert_eq!(card_size(&builtin_products()[1], false), Vec2 { x: 200.0, y: 60.0 });
}

#[test]
fn device_block_is_drawn_at_the_cards_size_with_a_second_strip_when_asked() {
    let p = wide();
    let top = |two: bool| {
        let mut n = 0;
        let b = device_block(
            &p,
            two,
            || {
                n += 1;
                Handle(n)
            },
            1.0,
        );
        let ys: Vec<f64> = b
            .entities
            .iter()
            .filter_map(|e| match &e.kind {
                EntityKind::LwPolyline(l) => Some(l.vertices.iter().map(|v| v.p.y.abs()).fold(0.0, f64::max)),
                _ => None,
            })
            .collect();
        (b.name, ys.into_iter().fold(0.0, f64::max))
    };
    let (one, h1) = top(false);
    let (two, h2) = top(true);
    assert_eq!((one.as_str(), two.as_str()), ("CSD_CHAMMQ50", "CSD_CHAMMQ50.2"));
    assert!((h1 - 45.0).abs() < 1.0 && (h2 - 54.0).abs() < 1.0, "{h1} {h2}");
    assert!(is_two_line_block(&two) && !is_two_line_block(&one));
}

#[test]
fn built_in_label_wraps_in_its_box() {
    let mut n = 0;
    let b = device_block(
        &builtin_products()[0],
        false,
        || {
            n += 1;
            Handle(n)
        },
        1.0,
    );
    let label = b.entities.iter().find_map(|e| match &e.kind {
        EntityKind::AttDef(a) if a.tag == "LABEL" => Some(a),
        _ => None,
    });
    let ml = label.and_then(|a| a.multiline.as_ref()).map(|m| m.mtext.width);
    assert_eq!(ml, Some(216.0 - 16.0));
}

#[test]
fn port_point_honours_rotation_and_scale() {
    let at = Vec2 { x: 100.0, y: 50.0 };
    // Right side: (35, 0) scaled x2 = (70, 0), rotated 90 degrees = (0, 70).
    let p = port_point(at, std::f64::consts::FRAC_PI_2, Vec2 { x: 2.0, y: 1.0 }, CARD, Side::Right, Slot::SOLO);
    assert!(p.near(Vec2 { x: 100.0, y: 120.0 }, 1e-9), "{p:?}");
}

#[test]
fn route_is_orthogonal_and_anchored() {
    let (a, b) = (Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.0, y: 70.0 });
    for (s1, s2) in [(Side::Right, Side::Left), (Side::Top, Side::Left), (Side::Right, Side::Bottom), (Side::Left, Side::Left)] {
        let r = route(a, s1, Slot::SOLO, b, s2, Slot { index: 1, total: 3, lane: 1 }, true);
        assert_eq!((r.first(), r.last()), (Some(&a), Some(&b)));
        for w in r.windows(2) {
            assert!(w[0].x == w[1].x || w[0].y == w[1].y, "{s1:?} {s2:?} {r:?}");
        }
    }
}

#[test]
fn smooth_route_is_anchored_leaves_perpendicular_and_is_not_orthogonal() {
    let (a, b) = (Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.0, y: 70.0 });
    let r = route(a, Side::Right, Slot::SOLO, b, Side::Left, Slot::SOLO, false);
    assert_eq!((r.first(), r.last()), (Some(&a), Some(&b)));
    assert_eq!(r.len(), 25);
    assert!(r.windows(2).any(|w| w[0].x != w[1].x && w[0].y != w[1].y));
    // Leaves the right port along +x and enters the left port along +x.
    assert!(r[1].x > a.x && (r[1].y - a.y).abs() < 0.1 * (r[1].x - a.x));
    assert!(b.x > r[23].x && (b.y - r[23].y).abs() < 0.1 * (b.x - r[23].x));
    assert_eq!(deco_points(&r, false, None).map(|d| d[2].1), r.get(12).copied());
}

#[test]
fn facing_stubs_never_double_back() {
    // Bottom of one card to the top of one below and to the left, outer lane: the source stub
    // reaches past the target's stub end.
    let (a, b) = (Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: -150.0, y: -40.0 });
    for (dx, lane) in [(-150.0, 2), (0.0, 0)] {
        let r = route(a, Side::Bottom, Slot { index: 0, total: 3, lane }, Vec2 { x: dx, ..b }, Side::Top, Slot::SOLO, true);
        for w in r.windows(3) {
            let (u, v) = (w[1] - w[0], w[2] - w[1]);
            assert!(u.dot(v) >= 0.0, "doubles back: {r:?}");
        }
        assert_eq!((r.first(), r.last()), (Some(&a), Some(&Vec2 { x: dx, ..b })));
    }
}

#[test]
fn device_block_has_both_attribute_definitions() {
    let mut n = 0;
    let b = device_block(
        &sample_products()[0],
        false,
        || {
            n += 1;
            Handle(n)
        },
        1.0,
    );
    assert_eq!(b.name, "CSD_CHAMMQ50");
    let defs: Vec<(String, bool, String)> = b
        .entities
        .iter()
        .filter_map(|e| match &e.kind {
            EntityKind::AttDef(a) => Some((a.tag.clone(), a.invisible, a.text.value.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        defs,
        [
            ("PRODUCT".into(), true, "CHAMMQ50".into()),
            ("NAME".into(), true, String::new()),
            ("NOTES".into(), true, String::new()),
            ("LABEL".into(), false, String::new())
        ]
    );
    assert_eq!(block_name("a b/c"), "CSD_a_b_c");
}

#[test]
fn world_side_turns_with_rotation() {
    let one = Vec2 { x: 1.0, y: 1.0 };
    let q = std::f64::consts::FRAC_PI_2;
    assert_eq!(world_side(Side::Right, q, one), Side::Top);
    assert_eq!(world_side(Side::Right, -q, one), Side::Bottom);
    assert_eq!(world_side(Side::Left, 0.0, Vec2 { x: -1.0, y: 1.0 }), Side::Right);
    assert_eq!(world_side(Side::Top, f64::NAN, one), Side::Top);
}

#[test]
fn three_wires_on_a_side_are_spread_and_ordered_by_far_end() {
    let (at, one) = (Vec2::ZERO, Vec2 { x: 1.0, y: 1.0 });
    let frame = edge_frame(at, 0.0, one, CARD, Side::Bottom);
    // Far ends listed out of order: right, left, middle.
    let far = [(1, Vec2 { x: 150.0, y: -150.0 }), (2, Vec2 { x: -150.0, y: -150.0 }), (3, Vec2 { x: 0.0, y: -150.0 })];
    let order = fan_order(&frame, &far);
    assert_eq!(order.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [2, 3, 1]);
    let xs: Vec<f64> = order.iter().map(|(_, s)| port_point(at, 0.0, one, CARD, Side::Bottom, *s).x).collect();
    assert_eq!(xs, [-17.5, 0.0, 17.5]);
    // One per direction here, so all in lane 0; their first legs start at different points.
    let lanes: Vec<usize> = order.iter().map(|(_, s)| s.lane).collect();
    assert_eq!(lanes, [0, 0, 0]);
    let first_legs: Vec<Vec2> = order
        .iter()
        .map(|(_, s)| {
            route(port_point(at, 0.0, one, CARD, Side::Bottom, *s), Side::Bottom, *s, Vec2 { x: 500.0, y: -500.0 }, Side::Top, Slot::SOLO, true)[1]
        })
        .collect();
    assert!(first_legs.windows(2).all(|w| !w[0].near(w[1], 1e-9)));
}

#[test]
fn labels_truncate_like_the_web_app() {
    assert_eq!(truncate_label("RR2XWASHNI", CARD_W), "RR2XWAS...");
    assert_eq!(truncate_label("Cyc Lights", CARD_W), "Cyc Lights");
    assert_eq!(truncate_label("FOH console", CARD_W), "FOH console");
}

fn with_name(name: &str, code: &str) -> Product {
    Product { id: code.into(), name: name.into(), code: code.into(), ..Product::default() }
}

#[test]
fn canvas_code_of_a_huge_name_is_fast() {
    let name = "Abcdefghij ".repeat(100_000);
    let t = std::time::Instant::now();
    let code = canvas_code(&with_name(&name, &"X".repeat(40)));
    assert!(code.chars().count() <= CANVAS_CODE_MAX);
    assert!(t.elapsed().as_secs() < 2);
}

#[test]
fn canvas_code_matches_the_web_apps_contraction() {
    for p in sample_products() {
        assert!(canvas_code(&p).chars().count() <= CANVAS_CODE_MAX, "{}", p.code);
    }
    // A code that fits is untouched; an override wins; the rest is contracted from the name.
    assert_eq!(canvas_code(&with_name("MagicQ", "MQ250M")), "MQ250M");
    assert_eq!(canvas_code(&with_name("EPIX Flex Drive", "EPIX FLEX DRIVE")), "EpixFLXDRV");
    for (name, code, short) in [
        ("Rogue Outcast 3X Wash", "ROGUEOUTCAST3XWASH", "ROC3XW"),
        ("Maverick Storm 2 BeamWash", "MAVERICKSTORM2BEAMWA", "MS2BW"),
        ("Maverick Storm 2 Profile", "MAVERICKSTORM2PROFIL", "MS2P"),
        ("Maverick Storm 4 Solo Wash", "MAVSTORM4SOLOWASH", "MS4SW"),
        ("ÉPIX Flex Boost", "EPIXFLEXBOOSTXX", "EPIFLEBOOS"),
    ] {
        assert_eq!(canvas_code(&with_name(name, code)), short, "{code}");
    }
    // The variant marker M survives while words are cut.
    let m = canvas_code(&with_name("Rogue Outcast 1 BeamWash M", "ROGUEOUTCAST1BEAMWAS2"));
    assert!(m.ends_with('M') && m != canvas_code(&with_name("Rogue Outcast 1 BeamWash", "ROGUEOUTCAST1BEAMWAS")), "{m}");
    // No name to contract: the code's first characters.
    assert_eq!(canvas_code(&with_name("", "ABCDEFGHIJKLMN")), "ABCDEFGHIJ");
}

#[test]
fn decoration_xdata_never_reads_as_a_wire() {
    let tags = deco_xdata(Handle(0x2a), Deco::Pill);
    assert_eq!(deco_from_xdata(&tags), Some((Handle(0x2a), Deco::Pill)));
    assert_eq!(WireLink::from_xdata(&tags), None);
    assert_eq!(deco_from_xdata(&tags[..2]), None);
}

#[test]
fn only_a_missing_product_gets_a_dashed_border() {
    let dashed = |p: &Product| {
        let mut n = 0;
        let b = device_block(
            p,
            false,
            || {
                n += 1;
                Handle(n)
            },
            9.0,
        );
        b.entities.iter().filter(|e| e.common.linetype == "DASHED2" && e.common.ltscale == 9.0).count()
    };
    let all = sample_products();
    assert_eq!(all.iter().filter(|p| dashed(p) == 1).map(|p| p.id.as_str()).collect::<Vec<_>>(), ["OVCYC1FCV2"]);
    assert_eq!(all.iter().filter(|p| p.missing).count(), 1);
}

// Skipped when the copy of the web app's bundled library (428 records, 13 MB) is not on this machine.
#[test]
fn bundled_library_round_trips() {
    let path = "/private/tmp/claude-503/-Users-davidgooch-orca-workspaces-cadcraft-csd/9495961c-914c-41f3-ad6b-5b52cd313405/scratchpad/library.json";
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let products = parse_library(&text).unwrap();
    assert_eq!(products.len(), 428);
    assert_eq!(products.iter().map(|p| &p.id).collect::<HashSet<_>>().len(), 428);
    assert!(products.iter().any(|p| p.extra.get("img").and_then(|v| v.as_str()).is_some_and(|i| !i.is_empty())));
    assert_eq!(parse_library(&library_json(&products)).unwrap(), products);
}

#[test]
fn hostile_library_input_is_an_error_or_defaults() {
    assert!(parse_library("{}").is_err() && parse_library("nope").is_err() && parse_library("").is_err());
    assert!(parse_library(&format!("[{}]", vec!["{}"; MAX_PRODUCTS + 1].join(","))).is_err());
    let p = parse_library(
        r##"[1, null, {"id": "", "name": "x"}, {"id": "a"}, {"id": "A", "name": "First", "dmxIn": -3, "dmxOut": 1e300, "netIn": "7", "netOut": [1],
        "poeWatts": -5, "channels": 1.9, "color": "red", "code": 12},
        {"id": "A", "name": "Second"}]"##,
    )
    .unwrap();
    assert_eq!(p.len(), 1);
    let p = &p[0];
    assert_eq!((p.name.as_str(), p.code.as_str(), p.cat.as_str(), p.color), ("First", "12", "Uncategorized", 0x555555));
    assert_eq!((p.ports(Protocol::Dmx), p.ports(Protocol::Network), p.poe_watts, p.channels), ((0, 1000), (7, 0), 0.0, 1));
}

#[test]
fn legible_darkens_light_brand_colours_only() {
    let luma = |c: u32| {
        let cadcraft_doc::color::Rgb(r, g, b) = cadcraft_doc::color::Rgb::from_u32(c);
        (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) / 255.0
    };
    assert_eq!(legible(0x1565c0), 0x1565c0);
    for light in [0x65edfa, 0xf7ae41, 0xffffff] {
        assert!((0.3..0.46).contains(&luma(legible(light))), "{light:x}");
    }
}

#[test]
fn waypoints_label_and_name_round_trip_and_old_wires_still_read() {
    let mut w = WireLink::new(Handle(0xAB), Side::Top, Handle(0x1F), Side::Bottom, Protocol::Dmx);
    w.points = vec![Vec2 { x: 1.5, y: -2.25 }, Vec2 { x: 1e-3, y: 4e5 }];
    w.label_t = Some(0.25);
    w.name = Some("Rack: 1, left".into());
    assert_eq!(WireLink::from_xdata(&w.to_xdata()), Some(w.clone()));

    // A wire saved before these existed carries only the five ends.
    let old = WireLink::new(Handle(0xAB), Side::Top, Handle(0x1F), Side::Bottom, Protocol::Dmx);
    assert_eq!(WireLink::from_xdata(&old.to_xdata()[..6]), Some(old));

    // Hostile extras: bad numbers are refused or dropped, the count is capped, t is clamped.
    let mut x = w.to_xdata();
    x.truncate(6);
    let tag = |v: String| RawTag { code: 1000, value: v };
    x.extend((0..MAX_WIRE_POINTS + 10).map(|i| tag(format!("pt:{i},0"))));
    x.extend([tag("lt:9".into()), tag("unknown".into())]);
    let read = WireLink::from_xdata(&x).unwrap();
    assert_eq!((read.points.len(), read.label_t), (MAX_WIRE_POINTS, Some(1.0)));
    for bad in ["pt:nan,1", "pt:inf,1", "pt:oops"] {
        let mut b = w.to_xdata();
        b.push(tag(bad.into()));
        assert_eq!(WireLink::from_xdata(&b).map(|l| l.points.len()), Some(2), "{bad}");
    }
}

#[test]
fn waypoints_beyond_the_coordinate_bound_are_dropped() {
    let mut x = link(1, 2, Protocol::Network).to_xdata();
    x.extend(["pt:1e308,0", "pt:0,-1e8", "pt:10,20"].map(|v| RawTag { code: 1000, value: v.into() }));
    let read = WireLink::from_xdata(&x).unwrap();
    assert_eq!(read.points, [Vec2 { x: 10.0, y: 20.0 }]);
}

#[test]
fn long_poe_and_rule_walks_stay_fast() {
    let net = [(Protocol::Network, 0, 4)];
    let supply = Product { poe_port_watts: 6000.0, ..prod(&net) };
    let lamp = Product { poe_watts: 0.001, ..prod(&net) };
    let n = 30_000u64;
    let devs: Vec<(Handle, &Product)> = (1..=n).map(|i| (Handle(i), if i == 1 { &supply } else { &lamp })).collect();
    let links: Vec<WireLink> = (1..n).map(|i| link(i, i + 1, Protocol::Network)).collect();
    let start = std::time::Instant::now();
    let _ = poe_verdict(&devs, &links, Handle(n), Handle(n - 1), 0.0);
    let _ = can_connect(&devs, &links, &link(n - 1, n, Protocol::Network));
    assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
}

#[test]
fn routes_pass_through_waypoints_in_both_modes() {
    let (a, b) = (Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.0, y: 70.0 });
    let via = [Vec2 { x: 40.0, y: 120.0 }, Vec2 { x: 60.0, y: -50.0 }];
    for ortho in [true, false] {
        let r = route_via(a, Side::Right, Slot::SOLO, b, Side::Left, Slot::SOLO, &via, ortho);
        assert_eq!((r.first(), r.last()), (Some(&a), Some(&b)));
        assert!(via.iter().all(|v| r.iter().any(|p| p.near(*v, 1e-6))), "ortho={ortho} {r:?}");
        if ortho {
            assert!(r.windows(2).all(|w| w[0].x == w[1].x || w[0].y == w[1].y));
        }
    }
    assert_eq!(
        route_via(a, Side::Right, Slot::SOLO, b, Side::Left, Slot::SOLO, &[], true),
        route(a, Side::Right, Slot::SOLO, b, Side::Left, Slot::SOLO, true)
    );
}

#[test]
fn an_ortho_waypoint_past_the_turn_does_not_double_back() {
    let (a, b, via) = (Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 300.0, y: 150.0 }, [Vec2 { x: 150.0, y: -150.0 }]);
    let r = route_via(a, Side::Bottom, Slot::SOLO, b, Side::Bottom, Slot::SOLO, &via, true);
    assert!(r.iter().any(|p| p.near(via[0], 1e-6)), "{r:?}");
    let reverses = r.windows(3).any(|w| {
        let (u, v) = (w[1] - w[0], w[2] - w[1]);
        (u.x * v.y - u.y * v.x).abs() < 1e-9 && u.dot(v) < 0.0
    });
    assert!(!reverses, "{r:?}");
}

#[test]
fn positions_along_a_path() {
    let p = [Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 30.0 }];
    assert!(point_at(&p, 0.25).unwrap().near(Vec2 { x: 10.0, y: 0.0 }, 1e-9));
    assert!(point_at(&p, 1.0).unwrap().near(p[2], 1e-9));
    assert!((fraction_near(&p, Vec2 { x: 12.0, y: 15.0 }, false).unwrap() - 0.625).abs() < 1e-9);
    assert!((fraction_near(&p, Vec2 { x: 12.0, y: 15.0 }, true).unwrap() - 0.625).abs() < 1e-9);
    assert!((fraction_near(&p, Vec2 { x: 5.0, y: 1.0 }, true).unwrap() - 0.125).abs() < 1e-9);
}

#[test]
fn builtins_take_any_protocol_and_end_a_10scene_bus() {
    let (by_others, off_page) = (&builtin_products()[0], &builtin_products()[1]);
    assert!(is_builtin(&by_others.id) && is_builtin(&off_page.id) && !is_builtin("logic-poex"));
    let fixture = prod(&[(Protocol::Dmx, 1, 1), (Protocol::Scene10, 1, 1), (Protocol::Midi, 1, 1)]);
    let devs = [(1, by_others), (2, &fixture), (3, off_page)];
    for p in Protocol::ALL {
        if fixture.effective_capacity(p) > 0 {
            assert_eq!(code(&devs, &[], &link(1, 2, p)), None, "{p:?}");
        }
        assert_eq!(code(&devs, &[], &link(1, 3, p)), None, "{p:?}");
    }
    assert_eq!(by_others.ports(Protocol::Scene10), (0, 99));
    assert_eq!(lookup(&[], BY_OTHERS_ID), Some(by_others));
    // A library row of the same id cannot shadow the built-in.
    let shadow = Product { id: BY_OTHERS_ID.into(), name: "other".into(), ..Product::default() };
    assert_eq!(lookup(&[shadow], BY_OTHERS_ID).map(|p| p.name.as_str()), Some("Supply by others"));
}

#[test]
fn builtin_card_is_a_dashed_box_without_a_strip() {
    let mut n = 0;
    let mut h = || {
        n += 1;
        Handle(n)
    };
    let plain = device_block(&sample_products()[0], false, &mut h, 1.0).entities;
    let by_others = device_block(&builtin_products()[0], false, &mut h, 1.0).entities;
    let off_page = device_block(&builtin_products()[1], false, &mut h, 1.0).entities;
    assert!(by_others.len() < plain.len());
    assert_eq!(off_page.len(), by_others.len() + 3, "the break arrow, the hidden page and the See line");
    assert!(by_others.iter().any(|e| e.common.linetype == "DASHED2"));
    assert!(plain.iter().all(|e| e.common.linetype != "DASHED2"));
}

fn named(code: &str, ports: &[(Protocol, u32, u32)]) -> Product {
    Product { name: code.into(), code: code.into(), ..prod(ports) }
}

/// Links `ids` in a chain and returns the rule id (if any) refusing `a`–`b`.
fn rule(devs: &[(u64, &Product)], chain: &[u64], a: u64, b: u64, protocol: Protocol) -> Option<&'static str> {
    let links: Vec<_> = chain.windows(2).map(|w| link(w[0], w[1], protocol)).collect();
    let l: Vec<_> = devs.iter().map(|(h, p)| (Handle(*h), *p)).collect();
    let r = can_connect(&l, &links, &link(a, b, protocol)).err()?;
    assert_eq!(r.code, "rule-violation", "{}", r.message);
    assert!(r.message.ends_with(&format!("[{}]", r.rule_id?)));
    r.rule_id
}

#[test]
fn rule_scn_bus_max10() {
    let origin = named("RACK", &[(Protocol::Scene10, 0, 4)]);
    let st = named("STN", &[(Protocol::Scene10, 1, 1)]);
    let mut devs = vec![(1, &origin)];
    devs.extend((2..=12).map(|h| (h, &st)));
    let chain: Vec<u64> = (1..=11).collect();
    assert_eq!(rule(&devs, &chain[..10], 10, 11, Protocol::Scene10), None);
    assert_eq!(rule(&devs, &chain, 11, 12, Protocol::Scene10), Some("scn-bus-max10"));
    // A second output of the same origin starts another bus.
    assert_eq!(rule(&devs, &chain, 1, 12, Protocol::Scene10), None);
}

#[test]
fn rule_logic_mr16_poe_x() {
    let poex = named("POE-X", &[(Protocol::Network, 0, 3)]);
    let mr16 = named("MR16", &[(Protocol::Network, 0, 2)]);
    let ar111 = Product { poe_watts: 45.0, ..named("AR111", &[(Protocol::Network, 0, 2)]) };
    let gs8 = named("GS8", &[(Protocol::Network, 0, 8)]);
    let devs = [(1, &poex), (2, &mr16), (3, &ar111), (4, &gs8)];
    let net = Protocol::Network;
    assert_eq!(rule(&devs, &[], 1, 2, net), None);
    assert_eq!(rule(&devs, &[], 1, 4, net), None);
    assert_eq!(rule(&devs, &[], 1, 3, net), Some("logic-mr16-poe-x"));
    assert_eq!(rule(&devs, &[], 3, 1, net), Some("logic-mr16-poe-x"));
    assert_eq!(rule(&devs, &[], 2, 4, net), Some("logic-mr16-poe-x"));
}

#[test]
fn rule_logic_mr16_max8() {
    let mr16 = named("MR16", &[(Protocol::Network, 0, 2)]);
    let devs: Vec<(u64, &Product)> = (1..=9).map(|h| (h, &mr16)).collect();
    let chain: Vec<u64> = (1..=8).collect();
    assert_eq!(rule(&devs, &chain[..7], 7, 8, Protocol::Network), None);
    assert_eq!(rule(&devs, &chain, 8, 9, Protocol::Network), Some("logic-mr16-max8"));
}

#[test]
fn rule_logic_fixture_net_thru() {
    let fixture = Product { poe_watts: 45.0, ..named("AR111", &[(Protocol::Network, 0, 1), (Protocol::NetData, 0, 1)]) };
    let mr16 = named("MR16", &[(Protocol::Network, 0, 1), (Protocol::NetData, 0, 1)]);
    let injector = named("INJ", &[(Protocol::NetData, 0, 2), (Protocol::Network, 0, 2)]);
    let devs = [(1, &fixture), (2, &injector), (3, &mr16)];
    let l: Vec<_> = devs.iter().map(|(h, p)| (Handle(*h), *p)).collect();
    let net = |links: &[WireLink], a, b| can_connect(&l, links, &link(a, b, Protocol::NetData)).err().and_then(|r| r.rule_id);
    assert_eq!(net(&[], 1, 2), Some("logic-fixture-net-thru"));
    assert_eq!(net(&[link(2, 1, Protocol::Network)], 1, 2), None);
    assert_eq!(net(&[], 3, 2), Some("logic-fixture-net-thru"));
}

#[test]
fn rule_epix_flex_max_per_output() {
    let flex = named("EPIX FLEX 20", &[(Protocol::Epix3p, 1, 1)]);
    let boost = named("EPIX FLEX BOOST", &[(Protocol::Epix3p, 1, 1)]);
    let devs: Vec<(u64, &Product)> = vec![(1, &flex), (2, &flex), (3, &flex), (4, &boost), (5, &flex), (6, &flex), (7, &boost), (8, &flex)];
    let e = Protocol::Epix3p;
    assert_eq!(rule(&devs, &[1], 1, 2, e), None);
    assert_eq!(rule(&devs, &[1, 2], 2, 3, e), Some("epix-flex-max-per-output"));
    // Drive -> 20 -> 20 -> Boost -> 20 -> 20 is legal; the Boost starts a new leg.
    assert_eq!(rule(&devs, &[1, 2, 4, 5], 5, 6, e), None);
    // Two Boosts in series do not make a third legal leg: five strips on one output.
    assert_eq!(rule(&devs, &[1, 2, 4, 5, 6, 7], 7, 8, e), Some("epix-flex-max-per-output"));
}

#[test]
fn rule_epix_line_max_per_output() {
    let line = named("EPIXL10IP", &[(Protocol::Epix4p, 1, 1)]);
    let half = named("EPIX LINE 10 IP S", &[(Protocol::Epix4p, 1, 1)]);
    let mut devs: Vec<(u64, &Product)> = (1..=11).map(|h| (h, &line)).collect();
    devs.push((12, &half));
    let chain: Vec<u64> = (1..=10).collect();
    let e = Protocol::Epix4p;
    assert_eq!(rule(&devs, &chain[..9], 9, 10, e), None);
    assert_eq!(rule(&devs, &chain, 10, 11, e), Some("epix-line-max-per-output"));
    // 9 full strips (900) + one half-length S (50) fits; the S adds 50, not 100.
    assert_eq!(rule(&devs, &chain[..9], 9, 12, e), None);
}

#[test]
fn poe_budget_warns_over_the_port_and_refuses_over_the_cap() {
    let net = [(Protocol::Network, 0, 4)];
    let supply = Product { poe_port_watts: 60.0, ..prod(&net) };
    let lamp = |w| Product { poe_watts: w, ..prod(&net) };
    let (a, b, c) = (lamp(45.0), lamp(45.0), lamp(100.0));
    let devs: Vec<(Handle, &Product)> = vec![(Handle(1), &supply), (Handle(2), &a), (Handle(3), &b), (Handle(4), &c)];
    let verdict =
        |links: &[WireLink], f: u64, t: u64, ceiling| poe_verdict(&devs, links, Handle(f), Handle(t), ceiling).map(|v| (v.refuse, v.message));

    // 45W on a 60W port is fine; a second 45W lamp daisy-chained on warns but still connects.
    let on_port = [link(1, 2, Protocol::Network)];
    assert_eq!(verdict(&[], 1, 2, 0.0), None);
    let (refuse, msg) = verdict(&on_port, 2, 3, 0.0).unwrap();
    assert!(!refuse && msg.contains("90.0W") && msg.contains("max 60W"), "{msg}");
    assert_eq!(code(&devs.iter().map(|(h, p)| (h.0, *p)).collect::<Vec<_>>(), &on_port, &link(2, 3, Protocol::Network)), None);

    // 45 + 100 is past the 110W cap: refused, with the ChamSys code.
    assert!(verdict(&on_port, 2, 4, 0.0).unwrap().0);
    let r = can_connect(&devs, &on_port, &link(2, 4, Protocol::Network)).unwrap_err();
    assert_eq!(r.code, "poe-budget-exceeded");

    // No supply on the run: warns against the library's best port only when told one.
    let island = verdict(&[link(2, 3, Protocol::Network)], 3, 4, 100.0);
    assert!(island.is_some());
    assert_eq!(verdict(&[], 2, 3, 100.0), None);

    // Data cables are not budgeted, and the schedule sees the loaded port.
    let two = [link(1, 2, Protocol::Network), link(2, 3, Protocol::Network)];
    assert!(port_overload(&devs, &two, Handle(1)).unwrap().contains("90.0W"));
    assert_eq!(port_overload(&devs, &on_port, Handle(1)), None);
    assert_eq!(max_supply_watts(&[supply.clone(), a.clone()]), 60.0);
}

#[test]
fn split_category_reads_family_and_sub() {
    let s = split_category;
    assert_eq!(s("Rogue — Wash"), ("Rogue".into(), "Wash".into()));
    assert_eq!(s("Rogue – Wash — Zoom"), ("Rogue".into(), "Wash — Zoom".into()));
    assert_eq!(s("Moving  Lights"), ("Moving Lights".into(), String::new()));
    assert_eq!(s("Rogue-Wash"), ("Rogue-Wash".into(), String::new()));
    assert_eq!(s(""), (String::new(), String::new()));
}

#[test]
fn product_key_collapses_whitespace_and_keeps_case() {
    assert_eq!(product_key("Rogue  R2 \t Wash"), "Rogue_R2_Wash");
    assert_eq!(product_key("a b"), product_key("a  b"));
    assert_ne!(product_key("MQ50"), product_key("mq50"));
    assert_eq!(product_key(""), "");
}
