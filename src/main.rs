use std::error::Error;
use std::fmt::Display;
use std::process::exit;

#[macro_use]
extern crate envconfig_derive;
#[macro_use]
extern crate log;

use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use envconfig::Envconfig;
use serde::Deserialize;
use reqwest::{StatusCode, Url};

#[derive(Envconfig)]
struct Config {
    #[envconfig(from = "AWAIR_API_KEY")]
    pub api_key: String,

    #[envconfig(from = "AWAIR_DEVICE_TYPE")]
    pub device_type: String,

    #[envconfig(from = "AWAIR_DEVICE_ID")]
    pub device_id: String,
}

#[derive(Debug)]
struct Value<T> {
    sensor: T,
    index: u8,
}

#[derive(Debug)]
enum Measurement {
    // Sensor: "temp"
    // Description: "Temperature"
    // Units: ˚C
    // Units Description: "degrees Celsius"
    // Range: -40–185
    Temperature(Value<i16>),

    // Sensor: "humid"
    // Description: "Relative Humidity"
    // Units: %
    // Units Description: "relateive humidity (RH%)"
    // Range: 0 – 100
    Humidity(Value<u8>),

    // Sensor: "co2"
    // Description: "Carbon Dioxide (CO₂)"
    // Units: ppm
    // Units Description: "parts per million"
    // Range: 0 – 5,000
    CO2(Value<u16>),

    // Sensor: "voc"
    // Description: "Total Volatile Organic Compounds (TVOCs)"
    // Units: ppb
    // Units Description: "parts per billion"
    // Range: 20 – 60,000
    VOC(Value<u16>),

    // Sensor: "dust"
    // Description: "Particulate Matter (PM - Aggregate Dust)"
    // Units: μg/m³
    // Units Description: "relateive humidity (RH%)"
    // Range: 0 – 250
    Dust(Value<u8>),

    // Sensor: "pm25"
    // Description: "Particulate Matter (PM2.5 - Fine Dust)"
    // Units: μg/m³
    // Units Description: "relateive humidity (RH%)"
    // Range: 0 – 1,000
    PM25(Value<u16>),
}

#[derive(Debug)]
struct DataPoint {
    timestamp: DateTime<Utc>,
    // score: u8,
    // measurements: Box<[Measurement]>
}

#[derive(Debug, Deserialize)]
struct AwairMeasurement {
    comp: String,
    value: f64,
}

#[derive(Debug, Deserialize)]
struct AwairDataPoint {
    timestamp: DateTime<Utc>,
    score: f64,
    sensors: Box<[AwairMeasurement]>,
    indices: Box<[AwairMeasurement]>,
}

#[derive(Debug, Deserialize)]
struct Response {
    data: Box<[AwairDataPoint]>
}

#[derive(Debug)]
struct InvalidResponse {
    response: reqwest::blocking::Response
}

impl Display for InvalidResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("invalid response: {}", self.response.status()))
    }
}

impl Error for InvalidResponse {}

fn latest_complete_five_second_period() -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let duration = Duration::minutes(5);
    let timestamp = now.timestamp();
    let upper_timestamp = timestamp - (timestamp % duration.num_seconds());
    let upper = Utc.timestamp(upper_timestamp, 0);
    let lower = upper - duration;
    (lower, upper)
}

fn run(config: Config) -> Result<(), Box<dyn Error>> {
    let (from, to) = latest_complete_five_second_period();
    debug!("fetching data from: {} to: {}", from, to);

    let endpoint = format!("https://developer-apis.awair.is/v1/users/self/devices/{}/{}/air-data/raw", config.device_type, config.device_id);
    let params = [
        ("from", from.to_rfc3339_opts(SecondsFormat::Secs, true)),
        ("to", to.to_rfc3339_opts(SecondsFormat::Secs, true)),
    ];

    let url = Url::parse_with_params(&endpoint, &params)?;

    let client = reqwest::blocking::Client::new();
    let request = client.get(url).bearer_auth(config.api_key);

    let response = request.send()?;

    if response.status() != StatusCode::OK {
        return Err(Box::new(InvalidResponse { response }))
    }

    let payload: Response = response.json()?;

    println!("{:?}", payload);

    Ok(())
}
fn main() {
    pretty_env_logger::init();

    let result: Result<(), Box<dyn Error>> = Config::init()
        .map_err(|err| Box::new(err) as Box<dyn Error>)
        .and_then(|config| {
            run(config)
        });

    match result {
        Ok(_) => exit(0),
        Err(err) => {
            error!("{}", err);
            exit(1)
        }
    }
}
