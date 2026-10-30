use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use reqwest::{Response, StatusCode};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{CanteenError, Result};

const PAGE_SIZE: &str = "1000";
const MAX_PAGES: u32 = 20;
const LIST_VALID_FOR: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Canteen {
    pub id: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Latitude and longitude.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
pub struct Day {
    pub date: NaiveDate,
    pub closed: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Meal {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub prices: Prices,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct Prices {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub students: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub employees: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pupils: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub others: Option<f64>,
}

pub struct OpenMensaClient {
    http: reqwest::Client,
    base: String,
    list: Mutex<Option<(Instant, Arc<Vec<Canteen>>)>>,
}

fn unreachable_service(error: reqwest::Error) -> CanteenError {
    CanteenError::new(format!(
        "OpenMensa could not be reached: {}",
        error.without_url()
    ))
}

fn pages_in(response: &Response) -> u32 {
    response
        .headers()
        .get("x-total-pages")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

impl OpenMensaClient {
    pub fn new(http: reqwest::Client, base: String) -> Self {
        Self {
            http,
            base,
            list: Mutex::default(),
        }
    }

    /// `None` when OpenMensa has nothing at this path.
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Option<Response>> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .query(query)
            .send()
            .await
            .map_err(unreachable_service)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        response
            .error_for_status()
            .map(Some)
            .map_err(unreachable_service)
    }

    async fn read<T: DeserializeOwned>(response: Response) -> Result<T> {
        response.json().await.map_err(unreachable_service)
    }

    async fn canteens(&self, filters: &[(&str, &str)]) -> Result<Vec<Canteen>> {
        let mut canteens = Vec::new();
        let mut last = 1;
        let mut page = 1;
        while page <= last.min(MAX_PAGES) {
            let number = page.to_string();
            let mut query = vec![("limit", PAGE_SIZE), ("page", number.as_str())];
            query.extend_from_slice(filters);
            let Some(response) = self.get("/canteens", &query).await? else {
                break;
            };
            last = pages_in(&response);
            canteens.extend(Self::read::<Vec<Canteen>>(response).await?);
            page += 1;
        }
        Ok(canteens)
    }

    /// OpenMensa has no search by name, so the whole list is kept for a day.
    pub async fn all_canteens(&self) -> Result<Arc<Vec<Canteen>>> {
        let mut list = self.list.lock().await;
        if let Some((fetched, canteens)) = list.as_ref()
            && fetched.elapsed() < LIST_VALID_FOR
        {
            return Ok(canteens.clone());
        }
        let canteens = Arc::new(self.canteens(&[]).await?);
        *list = Some((Instant::now(), canteens.clone()));
        Ok(canteens)
    }

    pub async fn canteens_near(
        &self,
        latitude: f64,
        longitude: f64,
        km: u16,
    ) -> Result<Vec<Canteen>> {
        self.canteens(&[
            ("near[lat]", &latitude.to_string()),
            ("near[lng]", &longitude.to_string()),
            ("near[dist]", &km.to_string()),
        ])
        .await
    }

    pub async fn canteen(&self, id: u32) -> Result<Option<Canteen>> {
        match self.get(&format!("/canteens/{id}"), &[]).await? {
            Some(response) => Self::read(response).await.map(Some),
            None => Ok(None),
        }
    }

    pub async fn days(&self, id: u32) -> Result<Vec<Day>> {
        match self.get(&format!("/canteens/{id}/days"), &[]).await? {
            Some(response) => Self::read(response).await,
            None => Ok(Vec::new()),
        }
    }

    /// A day the canteen has published nothing for may not exist in the API.
    pub async fn meals(&self, id: u32, date: NaiveDate) -> Result<Vec<Meal>> {
        let path = format!("/canteens/{id}/days/{date}/meals");
        match self.get(&path, &[]).await? {
            Some(response) => Self::read(response).await,
            None => Ok(Vec::new()),
        }
    }
}
