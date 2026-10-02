//! Live data for the questions `mimo-core` recognizes: the local clock, and
//! the weather from Open-Meteo (free, no API key). Weather is the only thing
//! here that touches the network; without a place in the question, the
//! location is approximated from the public IP (ipwho.is) and cached.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, Timelike};
use mimo_core::info::{format_date, format_time, format_weather};
use mimo_core::{Day, Lang, Question, WeatherReport};
use serde_json::Value;

const HTTP_TIMEOUT: Duration = Duration::from_secs(6);
/// People don't move far in an hour; no need to re-locate on every question.
const LOCATION_CACHE: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
struct Location {
    name: String,
    latitude: f64,
    longitude: f64,
}

pub fn answer(question: &Question, lang: Lang) -> Result<String, String> {
    let now = Local::now();
    match question {
        Question::Time => Ok(format_time(lang, now.hour(), now.minute())),
        Question::Date => Ok(format_date(
            lang,
            now.weekday().num_days_from_monday() as usize,
            now.day(),
            now.month() as usize,
        )),
        Question::Weather { day, place } => weather(lang, *day, place.as_deref()).map_err(|err| {
            eprintln!("[weather] {err}");
            match lang {
                Lang::Fr => "Météo indisponible pour le moment (connexion ?).".to_string(),
                Lang::En => "Weather is unavailable right now (offline?).".to_string(),
            }
        }),
    }
}

fn weather(lang: Lang, day: Day, place: Option<&str>) -> Result<String, String> {
    let location = match place {
        Some(place) => geocode(place, lang)?,
        None => current_location()?,
    };

    let json = get_json(
        ureq::get("https://api.open-meteo.com/v1/forecast")
            .query("latitude", location.latitude.to_string())
            .query("longitude", location.longitude.to_string())
            .query("current", "temperature_2m,weather_code")
            .query("daily", "weather_code,temperature_2m_max,temperature_2m_min")
            .query("timezone", "auto")
            .query("forecast_days", "2"),
    )?;

    let index = match day {
        Day::Today => 0,
        Day::Tomorrow => 1,
    };
    let daily = |field: &str| json["daily"][field][index].as_f64();
    let current = |field: &str| json["current"][field].as_f64();
    let missing = || "unexpected forecast response".to_string();

    let report = WeatherReport {
        place: location.name,
        day,
        now: (day == Day::Today).then(|| current("temperature_2m")).flatten(),
        code: match day {
            Day::Today => current("weather_code").or_else(|| daily("weather_code")),
            Day::Tomorrow => daily("weather_code"),
        }
        .ok_or_else(missing)? as i32,
        min: daily("temperature_2m_min").ok_or_else(missing)?,
        max: daily("temperature_2m_max").ok_or_else(missing)?,
    };
    Ok(format_weather(lang, &report))
}

fn geocode(place: &str, lang: Lang) -> Result<Location, String> {
    let json = get_json(
        ureq::get("https://geocoding-api.open-meteo.com/v1/search")
            .query("name", place)
            .query("count", "1")
            .query("language", if lang == Lang::Fr { "fr" } else { "en" }),
    )?;
    let first = &json["results"][0];
    Ok(Location {
        name: first["name"].as_str().ok_or_else(|| format!("unknown place {place:?}"))?.to_string(),
        latitude: first["latitude"].as_f64().ok_or("geocoding: no latitude")?,
        longitude: first["longitude"].as_f64().ok_or("geocoding: no longitude")?,
    })
}

fn current_location() -> Result<Location, String> {
    static CACHE: OnceLock<Mutex<Option<(Location, Instant)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Some((location, at)) = cache.lock().expect("location cache poisoned").as_ref() {
        if at.elapsed() < LOCATION_CACHE {
            return Ok(location.clone());
        }
    }

    let json = get_json(ureq::get("https://ipwho.is/"))?;
    if json["success"].as_bool() == Some(false) {
        return Err(format!("ip location failed: {}", json["message"]));
    }
    let location = Location {
        name: json["city"].as_str().unwrap_or("?").to_string(),
        latitude: json["latitude"].as_f64().ok_or("ip location: no latitude")?,
        longitude: json["longitude"].as_f64().ok_or("ip location: no longitude")?,
    };
    *cache.lock().expect("location cache poisoned") = Some((location.clone(), Instant::now()));
    Ok(location)
}

fn get_json(request: ureq::RequestBuilder<ureq::typestate::WithoutBody>) -> Result<Value, String> {
    let body = request
        .config()
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .call()
        .map_err(|err| err.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|err| err.to_string())?;
    serde_json::from_str(&body).map_err(|err| err.to_string())
}
