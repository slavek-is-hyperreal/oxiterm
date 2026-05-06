use serde::Deserialize;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct CurrentWeather {
    pub temp_c: f32,
    pub apparent_temp: f32,
    pub windspeed: f32,
    pub wind_direction: u16,
    pub weathercode: u8,
    pub humidity: u8,
    pub precipitation: f32,
    pub is_day: bool,
}

#[derive(Debug, Clone)]
pub struct DailyForecast {
    pub date: String,
    pub temp_max: f32,
    pub temp_min: f32,
    pub weathercode: u8,
    pub precipitation_sum: f32,
}

#[derive(Debug, Clone)]
pub struct WeatherData {
    pub current: CurrentWeather,
    pub daily: Vec<DailyForecast>,
    pub fetched_at: Instant,
}

#[derive(Deserialize)]
struct OpenMeteoResponse {
    current: CurrentData,
    daily: DailyData,
}

#[derive(Deserialize)]
struct CurrentData {
    temperature_2m: f32,
    apparent_temperature: f32,
    weathercode: u8,
    windspeed_10m: f32,
    winddirection_10m: u16,
    relative_humidity_2m: u8,
    precipitation: f32,
    is_day: u8,
}

#[derive(Deserialize)]
struct DailyData {
    time: Vec<String>,
    weathercode: Vec<u8>,
    temperature_2m_max: Vec<f32>,
    temperature_2m_min: Vec<f32>,
    precipitation_sum: Vec<f32>,
}

pub fn fetch_krakow() -> anyhow::Result<WeatherData> {
    let url = "https://api.open-meteo.com/v1/forecast?latitude=50.06&longitude=19.94&current=temperature_2m,apparent_temperature,weathercode,windspeed_10m,winddirection_10m,relative_humidity_2m,precipitation,is_day&daily=weathercode,temperature_2m_max,temperature_2m_min,precipitation_sum&timezone=Europe%2FWarsaw&forecast_days=7";
    
    let resp: OpenMeteoResponse = ureq::get(url)
        .call()?
        .into_json()?;

    let current = CurrentWeather {
        temp_c: resp.current.temperature_2m,
        apparent_temp: resp.current.apparent_temperature,
        windspeed: resp.current.windspeed_10m,
        wind_direction: resp.current.winddirection_10m,
        weathercode: resp.current.weathercode,
        humidity: resp.current.relative_humidity_2m,
        precipitation: resp.current.precipitation,
        is_day: resp.current.is_day == 1,
    };

    let mut daily = Vec::new();
    for i in 0..resp.daily.time.len() {
        daily.push(DailyForecast {
            date: resp.daily.time[i].clone(),
            temp_max: resp.daily.temperature_2m_max[i],
            temp_min: resp.daily.temperature_2m_min[i],
            weathercode: resp.daily.weathercode[i],
            precipitation_sum: resp.daily.precipitation_sum[i],
        });
    }

    Ok(WeatherData {
        current,
        daily,
        fetched_at: Instant::now(),
    })
}

pub fn weather_emoji(code: u8, is_day: bool) -> &'static str {
    match code {
        0 => if is_day { "☀️" } else { "🌙" },
        1..=3 => if is_day { "🌤️" } else { "☁️" },
        45 | 48 => "🌫️",
        51 | 53 | 55 | 56 | 57 => "🌦️",
        61 | 63 | 65 | 66 | 67 => "🌧️",
        71 | 73 | 75 | 77 => "❄️",
        80 | 81 | 82 => "🌧️",
        85 | 86 => "❄️",
        95 | 96 | 99 => "⛈️",
        _ => "❓",
    }
}

pub fn weather_description(code: u8) -> &'static str {
    match code {
        0 => "Czyste niebo",
        1..=3 => "Częściowe zachmurzenie",
        45 | 48 => "Mgła",
        51..=57 => "Mżawka",
        61..=67 => "Deszcz",
        71..=77 => "Śnieg",
        80..=82 => "Ulewa",
        85..=86 => "Zamieć śnieżna",
        95..=99 => "Burza",
        _ => "Nieznana",
    }
}
