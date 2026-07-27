use crate::aws_sigv4::SigV4Signer;
use crate::utils::response::json_response;
use cloudflare_shared::observability::trace_context::TraceContext;
use worker::*;

// POST /lambda/query — SigV4-signed proxy to Lambda Function URL
pub async fn handler(mut req: Request, env: &Env, tc: &TraceContext) -> Result<Response> {
    let lambda_url = match env.var("LAMBDA_URL") {
        Ok(v) => v.to_string(),
        Err(_) => return json_response(
            502, &serde_json::json!({"status":"error","error":"Lambda URL not configured"}),
        ),
    };
    let access_key = match env.var("AWS_ACCESS_KEY_ID") {
        Ok(v) => v.to_string(),
        Err(_) => return json_response(
            502, &serde_json::json!({"status":"error","error":"AWS_ACCESS_KEY_ID not configured"}),
        ),
    };
    let secret_key = match env.var("AWS_SECRET_ACCESS_KEY") {
        Ok(v) => v.to_string(),
        Err(_) => return json_response(
            502, &serde_json::json!({"status":"error","error":"AWS_SECRET_ACCESS_KEY not configured"}),
        ),
    };

    let body_text = req.text().await.unwrap_or_default();
    let body_bytes = body_text.as_bytes();

    let signer = SigV4Signer::new(&access_key, &secret_key);

    let now = js_sys::Date::new_0();
    let amz_date = format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        now.get_utc_full_year(),
        now.get_utc_month() + 1,
        now.get_utc_date(),
        now.get_utc_hours(),
        now.get_utc_minutes(),
        now.get_utc_seconds(),
    );

    let payload_hash = crate::aws_sigv4::sha256_hex(body_bytes);
    let auth_header = signer.sign_request("POST", &lambda_url, body_bytes, &amz_date);

    let out_headers = Headers::new();
    out_headers.set("Content-Type", "application/json")?;
    out_headers.set("x-amz-date", &amz_date)?;
    out_headers.set("x-amz-content-sha256", &payload_hash)?;
    out_headers.set("Authorization", &auth_header)?;
    out_headers.set("traceparent", &tc.to_traceparent())?;

    let mut init = RequestInit::new();
    init.with_method(Method::Post);
    init.with_headers(out_headers);
    init.with_body(Some(body_text.into()));

    let out_req = Request::new_with_init(&lambda_url, &init)?;
    let mut resp = Fetch::Request(out_req).send().await?;

    let resp_bytes = resp.bytes().await?;
    let resp_status = resp.status_code();
    let mut response = Response::from_bytes(resp_bytes)?;
    response = response.with_status(resp_status);
    Ok(response)
}