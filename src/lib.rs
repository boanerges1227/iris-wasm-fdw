#[allow(warnings)]
mod bindings;
mod convert;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value as JsonValue;

use bindings::{
    exports::supabase::wrappers::routines::Guest,
    supabase::wrappers::{
        http,
        types::{Cell, Context, FdwError, FdwResult, OptionsType, Row, TypeOid},
        utils,
    },
};

#[derive(Debug, Default)]
struct IrisFdw {
    api_url: String,
    namespace: String,
    username: String,
    password: String,
    src_rows: Vec<JsonValue>,
    src_idx: usize,
}

// Store the instance initialized by the host for the scan lifecycle.
static mut INSTANCE: *mut IrisFdw = std::ptr::null_mut::<IrisFdw>();

impl IrisFdw {
    // Initialize the FDW instance used by the host.
    fn init_instance() {
        let instance = Self::default();
        unsafe {
            INSTANCE = Box::leak(Box::new(instance));
        }
    }

    // Access the instance only after the host has called init.
    fn this_mut() -> &'static mut Self {
        unsafe { &mut (*INSTANCE) }
    }

    // Build the Atelier request with a normalized URL, Basic Auth and escaped SQL.
    fn query_request(&self, query: &str) -> http::Request {
        http::Request {
            method: http::Method::Post,
            url: format!(
                "{}/{}/action/query",
                self.api_url.trim_end_matches('/'),
                self.namespace
            ),
            headers: vec![
                ("content-type".into(), "application/json".into()),
                (
                    "authorization".into(),
                    format!(
                        "Basic {}",
                        STANDARD.encode(format!("{}:{}", self.username, self.password))
                    ),
                ),
            ],
            body: serde_json::json!({"query": query}).to_string(),
        }
    }

    // Discard cached rows and rewind the cursor before a new scan.
    fn reset_scan(&mut self) {
        self.src_rows.clear();
        self.src_idx = 0;
    }

    // Load rows only after validating the HTTP response and its JSON structure.
    fn load_response(&mut self, status: u16, body: &str) -> FdwResult {
        self.reset_scan();
        if !(200..300).contains(&status) {
            return Err(format!("IRIS HTTP status {status}"));
        }
        let response: JsonValue = serde_json::from_str(body).map_err(|e| e.to_string())?;
        // Atelier may report SQL errors with HTTP 200, so inspect its application status.
        if let Some(status) = response.get("status") {
            let status = status.as_object().ok_or("IRIS status is not an object")?;
            if let Some(errors) = status.get("errors") {
                let errors = errors
                    .as_array()
                    .ok_or("IRIS status.errors is not an array")?;
                if !errors.is_empty() {
                    return Err(format!(
                        "IRIS query failed: {}",
                        JsonValue::Array(errors.clone())
                    ));
                }
            }
            if let Some(summary) = status.get("summary") {
                let summary = summary
                    .as_str()
                    .ok_or("IRIS status.summary is not a string")?;
                if !summary.is_empty() {
                    return Err(format!("IRIS query failed: {summary}"));
                }
            }
        }
        let rows = response
            .get("result")
            .and_then(|r| r.get("content"))
            .ok_or("IRIS response has no result.content")?
            .as_array()
            .ok_or("IRIS result.content is not an array")?;
        if rows.iter().any(|row| !row.is_object()) {
            return Err("IRIS result.content contains a non-object row".into());
        }
        // Replace the cache only after the entire response has been validated.
        self.src_rows = rows.clone();
        Ok(())
    }

    // Convert a source field and report missing columns or invalid values.
    fn source_cell(row: &JsonValue, name: &str, oid: TypeOid) -> Result<Option<Cell>, FdwError> {
        convert::source_cell(row, name, oid)
    }
}

impl Guest for IrisFdw {
    fn host_version_requirement() -> String {
        // semver expression for Wasm FDW host version requirement
        // ref: https://docs.rs/semver/latest/semver/enum.Op.html
        "^0.1.0".to_string()
    }

    fn init(ctx: &Context) -> FdwResult {
        Self::init_instance();
        let this = Self::this_mut();

        let opts = ctx.get_options(OptionsType::Server);
        this.api_url = opts.require_or("api_url", "http://localhost:52773/api/atelier/v3");
        this.namespace = opts.require_or("namespace", "USER");
        this.username = opts.require_or("username", "_SYSTEM");
        this.password = opts.require_or("password", "SYS");

        Ok(())
    }

    // Execute the configured query and discard the previous scan state.
    fn begin_scan(ctx: &Context) -> FdwResult {
        let this = Self::this_mut();
        // Always start a new scan at the first row.
        this.reset_scan();

        let opts = ctx.get_options(OptionsType::Table);
        let query = opts.require("query")?;
        let req = this.query_request(&query);
        let resp = http::post(&req)?;
        // Check HTTP status before decoding to preserve authentication errors.
        http::error_for_status(&resp)?;
        this.load_response(resp.status_code, &resp.body)?;

        utils::report_info(&format!(
            "IRIS query response rows: {}",
            this.src_rows.len()
        ));

        Ok(())
    }

    // Advance the cursor only after all requested columns have been converted.
    fn iter_scan(ctx: &Context, row: &Row) -> Result<Option<u32>, FdwError> {
        let this = Self::this_mut();

        if this.src_idx >= this.src_rows.len() {
            return Ok(None);
        }

        let src_row = &this.src_rows[this.src_idx];
        for tgt_col in ctx.get_columns() {
            let tgt_col_name = tgt_col.name();
            let cell = Self::source_cell(src_row, &tgt_col_name, tgt_col.type_oid())?;

            row.push(cell.as_ref());
        }

        this.src_idx += 1;

        Ok(Some(0))
    }

    fn re_scan(_ctx: &Context) -> FdwResult {
        Err("re_scan on foreign table is not supported".to_owned())
    }

    // Release cached rows and rewind the cursor at the end of the scan.
    fn end_scan(_ctx: &Context) -> FdwResult {
        let this = Self::this_mut();
        this.reset_scan();
        Ok(())
    }

    fn begin_modify(_ctx: &Context) -> FdwResult {
        Err("modify on foreign table is not supported".to_owned())
    }

    fn insert(_ctx: &Context, _row: &Row) -> FdwResult {
        Ok(())
    }

    fn update(_ctx: &Context, _rowid: Cell, _row: &Row) -> FdwResult {
        Ok(())
    }

    fn delete(_ctx: &Context, _rowid: Cell) -> FdwResult {
        Ok(())
    }

    fn end_modify(_ctx: &Context) -> FdwResult {
        Ok(())
    }
}

bindings::export!(IrisFdw with_types_in bindings);
