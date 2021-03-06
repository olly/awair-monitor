#![warn(clippy::all)]

use std::error::Error;
use std::fmt::Display;
use std::process::exit;

#[macro_use]
extern crate envconfig_derive;
#[macro_use]
extern crate log;

use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use envconfig::Envconfig;
use failure::Fail;
use futures::stream;
use futures::stream::{StreamExt, TryStreamExt};
use futures::TryFutureExt;
use reqwest::{StatusCode, Url};
use serde::Deserialize;

#[derive(Envconfig)]
struct Config {
    #[envconfig(from = "AWAIR_API_KEY")]
    pub api_key: String,

    #[envconfig(from = "AWAIR_DEVICE_TYPE")]
    pub device_type: String,

    #[envconfig(from = "AWAIR_DEVICE_ID")]
    pub device_id: String,

    #[envconfig(from = "INFLUXDB_URL")]
    pub influx_db_url: String,

    #[envconfig(from = "INFLUXDB_USERNAME")]
    pub influx_db_username: Option<String>,

    #[envconfig(from = "INFLUXDB_PASSWORD", default = "")]
    pub influx_db_password: String,

    #[envconfig(from = "INFLUXDB_DATABASE")]
    pub influx_db_database: String,
}

#[derive(Debug, Deserialize, Eq, Hash, PartialEq)]
enum MeasurementType {
    // Sensor: "temp"
    // Description: "Temperature"
    // Units: ˚C
    // Units Description: "degrees Celsius"
    // Range: -40–185
    #[serde(rename = "temp")]
    Temperature,

    // Sensor: "humid"
    // Description: "Relative Humidity"
    // Units: %
    // Units Description: "relateive humidity (RH%)"
    // Range: 0 – 100
    #[serde(rename = "humid")]
    Humidity,

    // Sensor: "co2"
    // Description: "Carbon Dioxide (CO₂)"
    // Units: ppm
    // Units Description: "parts per million"
    // Range: 0 – 5,000
    #[serde(rename = "co2")]
    CO2,

    // Sensor: "voc"
    // Description: "Total Volatile Organic Compounds (TVOCs)"
    // Units: ppb
    // Units Description: "parts per billion"
    // Range: 20 – 60,000
    #[serde(rename = "voc")]
    VOC,

    // Sensor: "dust"
    // Description: "Particulate Matter (PM - Aggregate Dust)"
    // Units: μg/m³
    // Units Description: "relateive humidity (RH%)"
    // Range: 0 – 250
    #[serde(rename = "dust")]
    Dust,

    // Sensor: "pm25"
    // Description: "Particulate Matter (PM2.5 - Fine Dust)"
    // Units: μg/m³
    // Units Description: "relateive humidity (RH%)"
    // Range: 0 – 1,000
    #[serde(rename = "pm25")]
    PM25,
}

impl MeasurementType {
    fn field_name(&self) -> &'static str {
        match self {
            MeasurementType::Temperature => "temperature",
            MeasurementType::Humidity => "humidity",
            MeasurementType::CO2 => "CO2",
            MeasurementType::VOC => "VOC",
            MeasurementType::Dust => "dust",
            MeasurementType::PM25 => "PM25",
        }
    }
}

#[derive(Debug, Deserialize)]
struct Measurement {
    #[serde(rename = "comp")]
    kind: MeasurementType,
    value: f64,
}

#[derive(Debug, Deserialize)]
struct DataPoint {
    timestamp: DateTime<Utc>,
    score: f64,
    sensors: Box<[Measurement]>,
    indices: Box<[Measurement]>,
}

#[derive(Debug, Deserialize)]
struct Response {
    data: Box<[DataPoint]>,
}

#[derive(Debug)]
struct InvalidResponse {
    response: reqwest::Response,
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

async fn post_to_influxdb<'a, I: Iterator<Item = &'a DataPoint>>(
    config: Config,
    measurements: I,
) -> Result<(), Box<dyn Error>> {
    let mut influxdb_client =
        influxdb::Client::new(&config.influx_db_url, &config.influx_db_database);

    if let Some(username) = config.influx_db_username.as_ref().cloned() {
        let password = config.influx_db_password.clone();
        influxdb_client = influxdb_client.with_auth(username, password);
    }

    let influx_db_client = &influxdb_client;

    let influx_db_measurements = measurements.map(|measurement| {
        let mut influxdb_measurement =
            influxdb::WriteQuery::new(measurement.timestamp.into(), "awair");

        influxdb_measurement = influxdb_measurement.add_field("score", measurement.score);

        for sensor_measurement in measurement.sensors.iter() {
            let name = format!("{}.sensor", sensor_measurement.kind.field_name());
            influxdb_measurement = influxdb_measurement.add_field(name, sensor_measurement.value);
        }

        for index_measurement in measurement.indices.iter() {
            let name = format!("{}.index", index_measurement.kind.field_name());
            influxdb_measurement = influxdb_measurement.add_field(name, index_measurement.value);
        }

        let device_id = config.device_id.clone();
        influxdb_measurement = influxdb_measurement.add_tag("device_id", device_id);
        influxdb_measurement
    });

    stream::iter(influx_db_measurements)
        .map(Ok)
        .try_for_each_concurrent(10, |measurement| async move {
            influx_db_client
                .query(&measurement)
                .await
                .map(|_| ())
                .map_err(|err| Box::new(err.compat()) as Box<dyn Error>)
        })
        .await
}

async fn run(config: Config) -> Result<(), Box<dyn Error>> {
    let (from, to) = latest_complete_five_second_period();
    debug!("fetching data from: {} to: {}", from, to);

    let endpoint = format!(
        "https://developer-apis.awair.is/v1/users/self/devices/{}/{}/air-data/raw",
        config.device_type, config.device_id
    );
    let params = [
        ("from", from.to_rfc3339_opts(SecondsFormat::Secs, true)),
        ("to", to.to_rfc3339_opts(SecondsFormat::Secs, true)),
    ];

    let url = Url::parse_with_params(&endpoint, &params)?;

    let client = reqwest::Client::new();
    let request = client.get(url).bearer_auth(&config.api_key);

    let response = request.send().await?;

    if response.status() != StatusCode::OK {
        return Err(Box::new(InvalidResponse { response }));
    }

    let payload: Response = response.json().await?;

    post_to_influxdb(config, payload.data.iter()).await?;

    Ok(())
}

async fn load_config() -> Result<Config, Box<dyn Error>> {
    Config::init().map_err(|err| Box::new(err) as Box<dyn Error>)
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let result = load_config().and_then(run).await;

    match result {
        Ok(_) => exit(0),
        Err(err) => {
            error!("{}", err);
            exit(1)
        }
    }
}
