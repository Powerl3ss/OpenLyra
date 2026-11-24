//! Analyse et reconstruction sélective des blocs EXIF (structure TIFF).

use crate::model::{Entry, Group, Risk};
use exif::{Context, Field, In, Reader, Tag, Value};
use std::collections::BTreeMap;
use std::io::Cursor;

const POINTERS: &[Tag] = &[
    Tag::ExifIFDPointer,
    Tag::GPSInfoIFDPointer,
    Tag::InteropIFDPointer,
    Tag::JPEGInterchangeFormat,
    Tag::JPEGInterchangeFormatLength,
    Tag::StripOffsets,
    Tag::StripByteCounts,
    Tag::TileOffsets,
    Tag::TileByteCounts,
];

const DEVICE: &[Tag] = &[
    Tag::Make,
    Tag::Model,
    Tag::BodySerialNumber,
    Tag::LensMake,
    Tag::LensModel,
    Tag::LensSerialNumber,
    Tag::LensSpecification,
    Tag::Software,
    Tag::MakerNote,
    Tag::CameraOwnerName,
];

const DATES: &[Tag] = &[
    Tag::DateTime,
    Tag::DateTimeOriginal,
    Tag::DateTimeDigitized,
    Tag::OffsetTime,
    Tag::OffsetTimeOriginal,
    Tag::OffsetTimeDigitized,
    Tag::SubSecTime,
    Tag::SubSecTimeOriginal,
    Tag::SubSecTimeDigitized,
];

const AUTHOR: &[Tag] = &[
    Tag::Artist,
    Tag::Copyright,
    Tag::ImageDescription,
    Tag::UserComment,
    Tag::ImageUniqueID,
];

const DISPLAY: &[Tag] = &[
    Tag::Orientation,
    Tag::XResolution,
    Tag::YResolution,
    Tag::ResolutionUnit,
    Tag::ColorSpace,
    Tag::PixelXDimension,
    Tag::PixelYDimension,
    Tag::YCbCrPositioning,
    Tag::YCbCrSubSampling,
    Tag::YCbCrCoefficients,
    Tag::ExifVersion,
    Tag::FlashpixVersion,
    Tag::ComponentsConfiguration,
    Tag::Gamma,
    Tag::WhitePoint,
    Tag::PrimaryChromaticities,
    Tag::ReferenceBlackWhite,
    Tag::ImageWidth,
    Tag::ImageLength,
    Tag::BitsPerSample,
    Tag::Compression,
    Tag::PhotometricInterpretation,
    Tag::SamplesPerPixel,
    Tag::PlanarConfiguration,
    Tag::TransferFunction,
];

/// Balises « Windows » (XPTitle, XPComment, XPAuthor, XPKeywords, XPSubject).
fn is_xp_tag(t: Tag) -> bool {
    t.context() == Context::Tiff && (0x9c9b..=0x9c9f).contains(&t.number())
}

fn category(f: &Field) -> Option<&'static str> {
    if POINTERS.contains(&f.tag) {
        return None;
    }
    if f.ifd_num == In::THUMBNAIL {
        return Some("exif.thumb");
    }
    match f.tag.context() {
        Context::Gps => return Some("exif.gps"),
        Context::Interop => return Some("exif.display"),
        _ => {}
    }
    if DEVICE.contains(&f.tag) {
        Some("exif.device")
    } else if DATES.contains(&f.tag) {
        Some("exif.dates")
    } else if AUTHOR.contains(&f.tag) || is_xp_tag(f.tag) {
        Some("exif.author")
    } else if DISPLAY.contains(&f.tag) {
        Some("exif.display")
    } else if f.tag.context() == Context::Exif {
        if f.tag.description().is_some() {
            Some("exif.shooting")
        } else {
            Some("exif.other")
        }
    } else {
        Some("exif.other")
    }
}

fn label(t: Tag) -> String {
    let s = match t {
        _ if t == Tag::Make => "Fabricant de l'appareil",
        _ if t == Tag::Model => "Modèle de l'appareil",
        _ if t == Tag::BodySerialNumber => "Numéro de série du boîtier",
        _ if t == Tag::LensMake => "Fabricant de l'objectif",
        _ if t == Tag::LensModel => "Modèle de l'objectif",
        _ if t == Tag::LensSerialNumber => "Numéro de série de l'objectif",
        _ if t == Tag::LensSpecification => "Caractéristiques de l'objectif",
        _ if t == Tag::Software => "Logiciel",
        _ if t == Tag::MakerNote => "Données propriétaires du fabricant",
        _ if t == Tag::CameraOwnerName => "Propriétaire de l'appareil",
        _ if t == Tag::DateTime => "Date de modification",
        _ if t == Tag::DateTimeOriginal => "Date de prise de vue",
        _ if t == Tag::DateTimeDigitized => "Date de numérisation",
        _ if t == Tag::OffsetTime => "Fuseau horaire",
        _ if t == Tag::OffsetTimeOriginal => "Fuseau horaire de la prise de vue",
        _ if t == Tag::Artist => "Auteur",
        _ if t == Tag::Copyright => "Droits d'auteur",
        _ if t == Tag::ImageDescription => "Description",
        _ if t == Tag::UserComment => "Commentaire",
        _ if t == Tag::ImageUniqueID => "Identifiant unique de l'image",
        _ if t == Tag::GPSLatitude => "Latitude",
        _ if t == Tag::GPSLongitude => "Longitude",
        _ if t == Tag::GPSLatitudeRef => "Hémisphère (latitude)",
        _ if t == Tag::GPSLongitudeRef => "Hémisphère (longitude)",
        _ if t == Tag::GPSAltitude => "Altitude",
        _ if t == Tag::GPSTimeStamp => "Heure GPS",
        _ if t == Tag::GPSDateStamp => "Date GPS",
        _ if t == Tag::GPSImgDirection => "Direction de prise de vue",
        _ if t == Tag::GPSSpeed => "Vitesse",
        _ if t == Tag::ExposureTime => "Temps d'exposition",
        _ if t == Tag::FNumber => "Ouverture",
        _ if t == Tag::PhotographicSensitivity => "Sensibilité ISO",
        _ if t == Tag::FocalLength => "Focale",
        _ if t == Tag::Flash => "Flash",
        _ if t == Tag::Orientation => "Orientation",
        _ if t == Tag::XResolution => "Résolution horizontale",
        _ if t == Tag::YResolution => "Résolution verticale",
        _ => "",
    };
    if !s.is_empty() {
        return s.to_string();
    }
    if is_xp_tag(t) {
        return match t.number() {
            0x9c9b => "Titre (Windows)",
            0x9c9c => "Commentaire (Windows)",
            0x9c9d => "Auteur (Windows)",
            0x9c9e => "Mots-clés (Windows)",
            _ => "Sujet (Windows)",
        }
        .to_string();
    }
    let name = t.to_string();
    if name.starts_with("Tag(") {
        format!("Balise inconnue 0x{:04X}", t.number())
    } else {
        name
    }
}

fn value_text(f: &Field, exif: &exif::Exif) -> String {
    if f.tag == Tag::MakerNote {
        let n = match &f.value {
            Value::Undefined(v, _) => v.len(),
            Value::Byte(v) => v.len(),
            _ => 0,
        };
        return format!("Bloc binaire de {} octets (peut contenir un numéro de série)", n);
    }
    if f.tag == Tag::Orientation {
        if let Some(o) = f.value.get_uint(0) {
            return match o {
                1 => "Normale",
                2 => "Miroir horizontal",
                3 => "Pivotée de 180°",
                4 => "Miroir vertical",
                5 => "Miroir + rotation de 90°",
                6 => "Pivotée de 90° (sens horaire)",
                7 => "Miroir + rotation de 270°",
                8 => "Pivotée de 90° (sens antihoraire)",
                _ => "Inconnue",
            }
            .to_string();
        }
    }
    if is_xp_tag(f.tag) {
        if let Value::Byte(b) = &f.value {
            let u: Vec<u16> = b
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect();
            return String::from_utf16_lossy(&u);
        }
    }
    let raw_len = match &f.value {
        Value::Undefined(v, _) => Some(v.len()),
        Value::Byte(v) => Some(v.len()),
        _ => None,
    };
    if let Some(len) = raw_len {
        if len > 64 && f.tag != Tag::UserComment {
            return format!("Données binaires ({} octets)", len);
        }
    }
    let s = f.display_value().with_unit(exif).to_string();
    s.trim_matches('"').to_string()
}

fn gps_decimal(exif: &exif::Exif) -> Option<(f64, f64)> {
    let get = |tag: Tag| exif.get_field(tag, In::PRIMARY);
    let to_deg = |f: &Field| -> Option<f64> {
        if let Value::Rational(v) = &f.value {
            if v.len() >= 3 {
                return Some(v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0);
            }
        }
        None
    };
    let mut lat = to_deg(get(Tag::GPSLatitude)?)?;
    let mut lon = to_deg(get(Tag::GPSLongitude)?)?;
    if let Some(r) = get(Tag::GPSLatitudeRef) {
        if r.display_value().to_string().contains('S') {
            lat = -lat;
        }
    }
    if let Some(r) = get(Tag::GPSLongitudeRef) {
        if r.display_value().to_string().contains('W') {
            lon = -lon;
        }
    }
    if lat.is_finite() && lon.is_finite() && (lat != 0.0 || lon != 0.0) {
        Some((lat, lon))
    } else {
        None
    }
}

fn group_for(id: &str) -> Group {
    match id {
        "exif.gps" => Group::new(
            id,
            "Localisation GPS",
            "Coordonnées exactes de l'endroit où la photo a été prise.",
            Risk::High,
        ),
        "exif.device" => Group::new(
            id,
            "Appareil et logiciel",
            "Marque, modèle, numéros de série et logiciel utilisés pour créer ou retoucher l'image.",
            Risk::Medium,
        ),
        "exif.author" => Group::new(
            id,
            "Auteur et description",
            "Nom de l'auteur, droits, commentaires et identifiants enregistrés dans l'image.",
            Risk::High,
        ),
        "exif.dates" => Group::new(
            id,
            "Dates et heures",
            "Moment précis de la prise de vue et des modifications.",
            Risk::Medium,
        ),
        "exif.shooting" => Group::new(
            id,
            "Réglages de prise de vue",
            "Exposition, ouverture, ISO, flash, focale… Peu sensibles mais révèlent l'appareil utilisé.",
            Risk::Low,
        ),
        "exif.thumb" => Group::new(
            id,
            "Miniature intégrée",
            "Petite copie de l'image. Elle peut montrer la photo avant recadrage ou retouche.",
            Risk::Medium,
        ),
        "exif.display" => Group::new(
            id,
            "Orientation et paramètres d'affichage",
            "Informations techniques utiles à l'affichage (rotation, résolution, espace colorimétrique).",
            Risk::Low,
        )
        .not_recommended()
        .note("Supprimer l'orientation peut faire pivoter l'image à l'affichage."),
        _ => Group::new(
            "exif.other",
            "Autres balises EXIF",
            "Balises non standard ou propres à un logiciel.",
            Risk::Medium,
        ),
    }
}

const ORDER: &[&str] = &[
    "exif.gps",
    "exif.author",
    "exif.device",
    "exif.dates",
    "exif.thumb",
    "exif.other",
    "exif.shooting",
    "exif.display",
];

/// Analyse un bloc TIFF/EXIF brut et renvoie les groupes trouvés.
pub fn analyze(tiff: &[u8]) -> Vec<Group> {
    let exif = match Reader::new().read_raw(tiff.to_vec()) {
        Ok(e) => e,
        Err(_) => {
            return vec![Group::new(
                "exif.other",
                "Bloc EXIF illisible",
                "Un bloc EXIF est présent mais n'a pas pu être décodé. Il sera supprimé en entier.",
                Risk::Medium,
            )
            .entries(vec![Entry::new("Taille", format!("{} octets", tiff.len()))])];
        }
    };
    let mut map: BTreeMap<&'static str, Vec<Entry>> = BTreeMap::new();
    for f in exif.fields() {
        if let Some(cat) = category(f) {
            if cat == "exif.thumb" {
                continue;
            }
            map.entry(cat).or_default().push(Entry::new(label(f.tag), value_text(f, &exif)));
        }
    }
    if let Some((lat, lon)) = gps_decimal(&exif) {
        let v = map.entry("exif.gps").or_default();
        v.insert(
            0,
            Entry::new("Position", format!("{:.6}, {:.6}", lat, lon)).with_link(format!(
                "https://www.openstreetmap.org/?mlat={:.6}&mlon={:.6}#map=16/{:.6}/{:.6}",
                lat, lon, lat, lon
            )),
        );
    }
    if let Some(t) = thumbnail(&exif) {
        map.entry("exif.thumb")
            .or_default()
            .push(Entry::new("Miniature JPEG", format!("{} octets", t.len())));
    } else if exif.fields().any(|f| f.ifd_num == In::THUMBNAIL) {
        map.entry("exif.thumb")
            .or_default()
            .push(Entry::new("Miniature", "Présente (format non JPEG)"));
    }

    let mut groups = Vec::new();
    for id in ORDER {
        if let Some(entries) = map.remove(id) {
            let mut g = group_for(id).entries(entries);
            if *id == "exif.device"
                && g.entries.iter().any(|e| e.key.contains("série") || e.key.contains("Propriétaire"))
            {
                g.risk = Risk::High;
            }
            groups.push(g);
        }
    }
    groups
}

fn thumbnail(exif: &exif::Exif) -> Option<&[u8]> {
    let off = exif.get_field(Tag::JPEGInterchangeFormat, In::THUMBNAIL)?.value.get_uint(0)? as usize;
    let len = exif
        .get_field(Tag::JPEGInterchangeFormatLength, In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    exif.buf().get(off..off.checked_add(len)?)
}

/// Reconstruit le bloc TIFF en ne gardant que les catégories non sélectionnées.
/// Renvoie `Ok(None)` si plus rien ne doit être conservé.
pub fn rebuild(tiff: &[u8], remove: &dyn Fn(&str) -> bool) -> Result<Option<Vec<u8>>, String> {
    let exif = Reader::new().read_raw(tiff.to_vec()).map_err(|e| e.to_string())?;
    let keep_thumb = !remove("exif.thumb");
    let thumb = if keep_thumb { thumbnail(&exif) } else { None };

    let kept: Vec<&Field> = exif
        .fields()
        .filter(|f| match category(f) {
            None => false,
            Some("exif.thumb") => thumb.is_some(),
            Some(c) => !remove(c),
        })
        .collect();

    if !kept.iter().any(|f| f.ifd_num == In::PRIMARY) {
        return Ok(None);
    }

    let mut w = exif::experimental::Writer::new();
    for f in &kept {
        w.push_field(f);
    }
    if let Some(t) = thumb {
        w.set_jpeg(t, In::THUMBNAIL);
    }
    let mut buf = Cursor::new(Vec::new());
    w.write(&mut buf, exif.little_endian()).map_err(|e| e.to_string())?;
    Ok(Some(buf.into_inner()))
}
