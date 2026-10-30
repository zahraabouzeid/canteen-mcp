use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use canteen_mcp::config::Settings;
use canteen_mcp::{app, build_server, http_client};
use reqwest::Url;
use serde_json::{Value, json};
use tower::ServiceExt;

const CANTEENS_PAGE1: &str = include_str!("fixtures/canteens_page1.json");
const CANTEENS_PAGE2: &str = include_str!("fixtures/canteens_page2.json");
const MEALS: &str = include_str!("fixtures/meals.json");
const DAYS: &str = include_str!("fixtures/days.json");

const AHORN: u32 = 95;
const BROKEN: u32 = 500;

type Recorded = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

struct OpenMensa {
    base: String,
    requests: Recorded,
}

impl OpenMensa {
    async fn start() -> Self {
        let requests = Recorded::default();
        let router = Router::new().fallback(answer).with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free port");
        let base = format!("http://{}", listener.local_addr().expect("an address"));
        tokio::spawn(async move { axum::serve(listener, router).await });
        Self { base, requests }
    }

    fn recorded(&self) -> Vec<(String, HashMap<String, String>)> {
        self.requests.lock().expect("not poisoned").clone()
    }

    fn requests_to(&self, path: &str) -> usize {
        self.recorded()
            .iter()
            .filter(|(known, _)| known == path)
            .count()
    }
}

async fn answer(State(requests): State<Recorded>, request: Request) -> Response {
    let url = Url::parse(&format!("http://openmensa{}", request.uri())).expect("a valid URL");
    let path = url.path().to_owned();
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    requests
        .lock()
        .expect("not poisoned")
        .push((path.clone(), query.clone()));
    let json = |body: &'static str| ([(header::CONTENT_TYPE, "application/json")], body);
    match path.as_str() {
        "/canteens" => {
            let body = match query.get("page").map(String::as_str) {
                Some("2") => CANTEENS_PAGE2,
                _ => CANTEENS_PAGE1,
            };
            ([("x-total-pages", "2")], json(body)).into_response()
        }
        "/canteens/95" => {
            json(r#"{"id": 95, "name": "Aachen, Mensa Ahorn", "city": "Aachen"}"#).into_response()
        }
        "/canteens/500" => json(r#"{"id": 500, "name": "Broken"}"#).into_response(),
        "/canteens/95/days" => json(DAYS).into_response(),
        "/canteens/95/days/2026-10-30/meals" => json(MEALS).into_response(),
        "/canteens/95/days/2026-11-03/meals" => json("[]").into_response(),
        "/canteens/500/days/2026-10-30/meals" => StatusCode::BAD_GATEWAY.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn post(router: &Router, host: &str, body: Value) -> (StatusCode, Value) {
    let request = axum::http::Request::post("/mcp")
        .header(header::HOST, host)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::CONTENT_TYPE, "application/json")
        .header("mcp-protocol-version", "2025-06-18")
        .body(Body::from(body.to_string()))
        .expect("a valid request");
    let response = router.clone().oneshot(request).await.expect("a response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

struct Server {
    openmensa: OpenMensa,
    router: Router,
    output_schemas: HashMap<String, Value>,
}

impl Server {
    async fn start() -> Self {
        let openmensa = OpenMensa::start().await;
        let settings = Settings::from_map(&HashMap::from([
            ("OPENMENSA_URL".to_owned(), openmensa.base.clone()),
            ("PUBLIC_HOST".to_owned(), "testserver".to_owned()),
        ]))
        .expect("valid settings");
        let router = app(&settings, build_server(&settings, http_client()));
        let (_, listed) = post(
            &router,
            "testserver",
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .await;
        let output_schemas = listed["result"]["tools"]
            .as_array()
            .expect("a tool list")
            .iter()
            .map(|tool| {
                let name = tool["name"].as_str().expect("a name").to_owned();
                (name, tool["outputSchema"].clone())
            })
            .collect();
        Self {
            openmensa,
            router,
            output_schemas,
        }
    }

    async fn call_raw(&self, name: &str, arguments: Value) -> Value {
        let (status, response) = post(
            &self.router,
            "testserver",
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{response}");
        response["result"].clone()
    }

    async fn call(&self, name: &str, arguments: Value) -> Value {
        let result = self.call_raw(name, arguments).await;
        assert_eq!(result["isError"], false, "{result}");
        let content = result["structuredContent"].clone();
        let validator =
            jsonschema::validator_for(&self.output_schemas[name]).expect("a valid output schema");
        if let Err(error) = validator.validate(&content) {
            panic!("{name} answered outside its output schema: {error}\n{content:#}");
        }
        content
    }

    async fn error(&self, name: &str, arguments: Value) -> String {
        let result = self.call_raw(name, arguments).await;
        assert_eq!(result["isError"], true, "{result}");
        result["content"][0]["text"]
            .as_str()
            .expect("an error text")
            .to_owned()
    }
}

#[tokio::test]
async fn find_canteens_searches_every_page_by_name_and_city() {
    let server = Server::start().await;
    let result = server
        .call("find_canteens", json!({"query": "köln MENSA"}))
        .await;
    assert_eq!(result["total"], 1);
    assert_eq!(result["canteens"][0]["id"], 200);
    assert_eq!(result["canteens"][0]["city"], "Köln");
    assert!(result["canteens"][0].get("coordinates").is_none());

    let result = server
        .call("find_canteens", json!({"query": "aachen"}))
        .await;
    assert_eq!(result["total"], 2);
    let result = server
        .call("find_canteens", json!({"query": "paris"}))
        .await;
    assert_eq!(result["total"], 0);
}

#[tokio::test]
async fn find_canteens_keeps_the_list_instead_of_asking_again() {
    let server = Server::start().await;
    server
        .call("find_canteens", json!({"query": "aachen"}))
        .await;
    server.call("find_canteens", json!({"query": "köln"})).await;
    assert_eq!(server.openmensa.requests_to("/canteens"), 2);
}

#[tokio::test]
async fn find_canteens_asks_around_a_point_and_can_narrow_by_name() {
    let server = Server::start().await;
    let result = server
        .call(
            "find_canteens",
            json!({"query": "ahorn", "near": "50.78, 6.07", "radius_km": 3}),
        )
        .await;
    assert_eq!(result["total"], 1);
    assert_eq!(result["canteens"][0]["id"], AHORN);

    let recorded = server.openmensa.recorded();
    let (_, query) = recorded.first().expect("a request");
    assert_eq!(query["near[lat]"], "50.78");
    assert_eq!(query["near[lng]"], "6.07");
    assert_eq!(query["near[dist]"], "3");
}

#[tokio::test]
async fn find_canteens_needs_something_to_search_for() {
    let server = Server::start().await;
    let error = server.error("find_canteens", json!({"query": "  "})).await;
    assert!(error.contains("Give a query"), "{error}");
    let error = server
        .error("find_canteens", json!({"near": "aachen"}))
        .await;
    assert!(error.contains("latitude,longitude"), "{error}");
    let error = server.error("find_canteens", json!({"near": "95,6"})).await;
    assert!(error.contains("latitude,longitude"), "{error}");
    let error = server
        .error("find_canteens", json!({"near": "50,6", "radius_km": 0}))
        .await;
    assert!(error.contains("radius_km"), "{error}");
}

#[tokio::test]
async fn canteen_menu_lists_meals_with_prices_and_notes() {
    let server = Server::start().await;
    let result = server
        .call(
            "canteen_menu",
            json!({"canteen_id": AHORN, "date": "2026-10-30"}),
        )
        .await;
    assert_eq!(result["canteen"]["name"], "Aachen, Mensa Ahorn");
    assert_eq!(result["date"], "2026-10-30");
    assert_eq!(result["weekday"], "Friday");
    assert!(result.get("note").is_none());

    let (first, second) = (&result["meals"][0], &result["meals"][1]);
    assert_eq!(first["name"], "Gemüse-Jambalaya | Reis");
    assert_eq!(first["category"], "Vegetarisch");
    assert_eq!(first["prices"]["students"], 2.5);
    assert_eq!(first["prices"]["others"], 4.0);
    assert!(first["prices"].get("pupils").is_none());
    assert_eq!(first["notes"][2], "vegan");
    assert_eq!(second["prices"], json!({}));
    assert!(second.get("notes").is_none());
}

#[tokio::test]
async fn canteen_menu_asks_for_today_without_a_date() {
    let server = Server::start().await;
    let result = server
        .call("canteen_menu", json!({"canteen_id": AHORN}))
        .await;
    let date = result["date"].as_str().expect("a date");
    let path = format!("/canteens/95/days/{date}/meals");
    assert_eq!(server.openmensa.requests_to(&path), 1);
}

#[tokio::test]
async fn canteen_menu_explains_a_day_without_meals() {
    let server = Server::start().await;
    let closed = server
        .call(
            "canteen_menu",
            json!({"canteen_id": AHORN, "date": "2026-11-03"}),
        )
        .await;
    assert_eq!(closed["meals"], json!([]));
    assert_eq!(closed["note"], "The canteen is closed on this day.");

    let unpublished = server
        .call(
            "canteen_menu",
            json!({"canteen_id": AHORN, "date": "2026-12-24"}),
        )
        .await;
    assert_eq!(unpublished["meals"], json!([]));
    let note = unpublished["note"].as_str().expect("a note");
    assert!(note.contains("canteen_days"), "{note}");
}

#[tokio::test]
async fn canteen_menu_refuses_what_it_cannot_answer() {
    let server = Server::start().await;
    let error = server
        .error(
            "canteen_menu",
            json!({"canteen_id": 7, "date": "2026-10-30"}),
        )
        .await;
    assert!(error.contains("no canteen with the id 7"), "{error}");

    let error = server
        .error(
            "canteen_menu",
            json!({"canteen_id": AHORN, "date": "tomorrow"}),
        )
        .await;
    assert!(error.contains("is not a date"), "{error}");

    let error = server
        .error(
            "canteen_menu",
            json!({"canteen_id": BROKEN, "date": "2026-10-30"}),
        )
        .await;
    assert!(error.contains("OpenMensa could not be reached"), "{error}");
    assert!(!error.contains("127.0.0.1"), "{error}");
}

#[tokio::test]
async fn canteen_days_are_in_order_and_say_what_today_is() {
    let server = Server::start().await;
    let result = server
        .call("canteen_days", json!({"canteen_id": AHORN}))
        .await;
    assert!(result["today"].is_string());
    let days = result["days"].as_array().expect("days");
    assert_eq!(days[0]["date"], "2026-10-30");
    assert_eq!(days[0]["weekday"], "Friday");
    assert_eq!(days[2]["date"], "2026-11-03");
    assert_eq!(days[2]["closed"], true);
}

#[tokio::test]
async fn every_tool_is_marked_read_only() {
    let server = Server::start().await;
    let (_, listed) = post(
        &server.router,
        "testserver",
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let tools = listed["result"]["tools"].as_array().expect("a tool list");
    assert_eq!(tools.len(), 3);
    for tool in tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
    }
}

#[tokio::test]
async fn another_host_is_refused() {
    let server = Server::start().await;
    let (status, _) = post(
        &server.router,
        "evil.example",
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_health_check_answers() {
    let server = Server::start().await;
    let request = axum::http::Request::get("/health")
        .body(Body::empty())
        .expect("a valid request");
    let response = server
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("a response");
    assert_eq!(response.status(), StatusCode::OK);
}
