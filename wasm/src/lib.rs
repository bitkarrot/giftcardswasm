use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

wit_bindgen::generate!({
    path: "wit/world.wit",
    world: "giftcards",
});

use lnbits::extension::host;

// ---------------------------------------------------------------------------
// Host API helpers — thin wrappers around wit-bindgen generated functions
// ---------------------------------------------------------------------------

fn h_storage_get(table: &str, id: &str) -> Option<Value> {
    let resp = host::storage_get(&host::StorageGetRequest {
        table: table.to_string(),
        id: id.to_string(),
    });
    resp.data_json.and_then(|s: String| serde_json::from_str(&s).ok())
}

fn h_storage_get_public(table: &str, id: &str) -> Option<Value> {
    let resp = host::storage_get_public(&host::StorageGetPublicRequest {
        table: table.to_string(),
        id: id.to_string(),
    });
    resp.data_json.and_then(|s: String| serde_json::from_str(&s).ok())
}

fn h_storage_get_public_paginated(
    table: &str,
    filters: &Value,
    limit: u32,
    offset: u32,
) -> (Vec<Value>, u32) {
    let filters_json = serde_json::to_string(filters).unwrap_or_default();
    let resp = host::storage_get_public_paginated(&host::StoragePublicPaginatedRequest {
        table: table.to_string(),
        filters_json: Some(filters_json),
        limit,
        offset,
    });
    let rows: Vec<Value> = serde_json::from_str(&resp.rows_json).unwrap_or_default();
    (rows, resp.total)
}

fn h_storage_set(table: &str, data: &Value) -> bool {
    let data_json = serde_json::to_string(data).unwrap_or_default();
    let resp = host::storage_set(&host::StorageSetRequest {
        table: table.to_string(),
        data_json: Some(data_json),
    });
    resp.ok
}

fn h_storage_delete(table: &str, id: &str) -> bool {
    let resp = host::storage_delete(&host::StorageDeleteRequest {
        table: table.to_string(),
        id: id.to_string(),
    });
    resp.ok
}

fn h_storage_get_paginated(
    table: &str,
    filters: &Value,
    limit: u32,
    offset: u32,
) -> (Vec<Value>, u32) {
    let filters_json = serde_json::to_string(filters).unwrap_or_default();
    let resp = host::storage_get_paginated(&host::StoragePaginatedRequest {
        table: table.to_string(),
        filters_json: Some(filters_json),
        search: None,
        search_fields_json: None,
        sort_by: None,
        descending: false,
        limit,
        offset,
    });
    let rows: Vec<Value> = serde_json::from_str(&resp.rows_json).unwrap_or_default();
    (rows, resp.total)
}

fn h_pay_invoice(wallet_id: &str, payment_request: &str, max_sat: u64) -> Value {
    let resp = host::pay_invoice(&host::PayInvoiceRequest {
        wallet_id: wallet_id.to_string(),
        payment_request: payment_request.to_string(),
        max_sat: Some(max_sat),
        description: String::new(),
    });
    json!({
        "ok": resp.ok,
        "error": resp.error,
        "checkingId": resp.checking_id,
        "paymentHash": resp.payment_hash,
        "status": resp.status,
        "pending": resp.pending,
        "success": resp.success
    })
}

fn h_wallet_balance(wallet_id: &str) -> Option<Value> {
    let resp = host::wallet_balance(&host::WalletBalanceRequest {
        wallet_id: wallet_id.to_string(),
    });
    Some(json!({
        "balanceSat": resp.balance_sat,
        "canSendPayments": resp.can_send_payments
    }))
}

fn h_list_wallets() -> Vec<(String, String)> {
    let resp = host::list_user_wallets();
    resp.wallets
        .iter()
        .map(|w| (w.id.clone(), w.name.clone()))
        .collect()
}

fn resolve_user_wallet(req: &Value) -> Result<String, String> {
    let wallets = h_list_wallets();
    if wallets.is_empty() {
        return Err("No wallet available for this user".to_string());
    }

    match req.get("walletId").and_then(|w| w.as_str()) {
        Some(wallet_id) if !wallet_id.is_empty() => {
            if wallets.iter().any(|(id, _)| id == wallet_id) {
                Ok(wallet_id.to_string())
            } else {
                Err("Wallet is not available to this user".to_string())
            }
        }
        _ => Ok(wallets[0].0.clone()),
    }
}

fn owned_card(card_id: &str) -> Option<Value> {
    let user_wallet_ids: Vec<String> = h_list_wallets()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    if user_wallet_ids.is_empty() {
        return None;
    }

    let filters = json!({"cardId": card_id});
    let (rows, _) = h_storage_get_paginated("cards", &filters, 1, 0);
    rows.into_iter().find(|card| {
        card.get("walletId")
            .and_then(|wallet_id| wallet_id.as_str())
            .map(|wallet_id| user_wallet_ids.iter().any(|id| id == wallet_id))
            .unwrap_or(false)
    })
}

fn sanitize_base_url(base: &str) -> String {
    if (base.starts_with("https://") || base.starts_with("http://"))
        && !base.chars().any(|c| c.is_whitespace() || c == '@')
    {
        base.trim_end_matches('/').to_string()
    } else {
        String::new()
    }
}

fn trusted_base_url(req: &Value) -> String {
    req.get("__baseUrl")
        .and_then(|b| b.as_str())
        .map(sanitize_base_url)
        .unwrap_or_default()
}

fn creation_base_url(req: &Value) -> String {
    let injected = trusted_base_url(req);
    if !injected.is_empty() {
        return injected;
    }

    // The current LNbits WASM adapter does not inject __baseUrl into guest
    // payloads. Accept the browser-provided origin for card creation only,
    // after the same scheme/authority validation used for stored URLs.
    req.get("baseUrl")
        .and_then(|b| b.as_str())
        .map(sanitize_base_url)
        .unwrap_or_default()
}

fn h_http_request(method: &str, url: &str, body: Option<&str>) -> Value {
    let resp = host::http_request(&host::HttpRequestParams {
        method: method.to_string(),
        url: url.to_string(),
        headers: vec![],
        body: body.map(|b| b.to_string()),
    });
    json!({
        "statusCode": resp.status_code,
        "body": resp.body
    })
}

fn h_now() -> u64 {
    host::now().timestamp
}

fn h_random_id(prefix: &str) -> String {
    host::random_id(&host::RandomIdRequest {
        prefix: prefix.to_string(),
    })
    .id
}

fn h_log(level: &str, msg: &str) {
    let _ = host::log(&host::LogRequest {
        level: level.to_string(),
        message: msg.to_string(),
    });
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn err(msg: &str) -> String {
    json!({ "error": msg }).to_string()
}

fn ok(data: Value) -> String {
    data.to_string()
}

/// SHA-256 of a byte slice, returned as a lowercase hex string.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let bytes = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in bytes.iter() {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

/// Gather `n` random bytes by hashing entropy from the host's random_id + clock.
/// The host random_id is the only source of randomness available to the guest.
fn random_bytes(n: usize) -> Vec<u8> {
    let mut entropy = String::new();
    // Each random_id yields ~32+ hex chars; gather more than enough entropy.
    while entropy.len() < 128 {
        entropy.push_str(&h_random_id("r"));
        entropy.push_str(&h_now().to_string());
    }
    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(entropy.as_bytes());
        hasher.finalize()
    };
    // One SHA-256 yields 32 bytes; for n <= 32 this suffices.
    let mut out: Vec<u8> = hash.iter().copied().collect();
    // Extend if more than 32 bytes are requested (rare).
    let mut counter: u64 = 0;
    while out.len() < n {
        counter += 1;
        let mut hasher = Sha256::new();
        hasher.update(&out);
        hasher.update(&counter.to_le_bytes());
        let extra = hasher.finalize();
        out.extend_from_slice(&extra);
    }
    out.truncate(n);
    out
}

/// Generate a URL-safe base64 magic token from 32 random bytes (no padding).
fn generate_magic_token() -> String {
    let bytes = random_bytes(32);
    URL_SAFE_NO_PAD.encode(&bytes)
}

// ---------------------------------------------------------------------------
// Bech32 encoding (for LNURL)
// ---------------------------------------------------------------------------

const BECH32_CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn bech32_polymod(values: &[u8]) -> u32 {
    let mut chk: u32 = 1;
    let generator: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    for &v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ffffff) << 5) ^ (v as u32);
        for i in 0..5 {
            if (top >> i) & 1 != 0 {
                chk ^= generator[i];
            }
        }
    }
    chk
}

fn bech32_hrp_expand(hrp: &[u8]) -> Vec<u8> {
    let mut ret = Vec::with_capacity(hrp.len() * 2 + 1);
    for &c in hrp {
        ret.push(c >> 5);
    }
    ret.push(0);
    for &c in hrp {
        ret.push(c & 31);
    }
    ret
}

fn bech32_create_checksum(hrp: &[u8], data: &[u8], constant: u32) -> Vec<u8> {
    let mut values = bech32_hrp_expand(hrp);
    values.extend_from_slice(data);
    values.extend_from_slice(&[0u8; 6]); // 6 zero bytes for checksum extraction
    let polymod = bech32_polymod(&values) ^ constant;
    let mut ret = Vec::with_capacity(6);
    for i in 0..6 {
        ret.push(((polymod >> (5 * (5 - i))) & 31) as u8);
    }
    ret
}

/// Convert a byte slice from `from`-bit groups to `to`-bit groups (with padding).
fn convert_bits(data: &[u8], from: u32, to: u32, pad: bool) -> Vec<u8> {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let maxv = (1u32 << to) - 1;
    let mut ret = Vec::new();
    for &value in data {
        let v = value as u32;
        if v >> from != 0 {
            continue;
        }
        acc = (acc << from) | v;
        bits += from;
        while bits >= to {
            bits -= to;
            ret.push(((acc >> bits) & maxv) as u8);
        }
    }
    if pad && bits > 0 {
        ret.push(((acc << (to - bits)) & maxv) as u8);
    }
    ret
}

/// Encode data with a given human-readable part using bech32 (constant = 1).
/// The result is uppercased to match the LNURL convention.
fn bech32_encode(hrp: &str, data: &[u8]) -> String {
    let hrp_bytes = hrp.as_bytes();
    let checksum = bech32_create_checksum(hrp_bytes, data, 1);
    let mut combined = data.to_vec();
    combined.extend_from_slice(&checksum);
    let mut result = String::with_capacity(hrp.len() + 1 + combined.len());
    result.push_str(hrp);
    result.push('1');
    for &v in &combined {
        let c = BECH32_CHARSET[(v as usize) % BECH32_CHARSET.len()];
        result.push(c.to_ascii_uppercase() as char);
    }
    result
}

/// Encode a URL string into LNURL bech32 format (HRP "LNURL", uppercased).
fn lnurl_encode(url: &str) -> String {
    let data = convert_bits(url.as_bytes(), 8, 5, true);
    bech32_encode("lnurl", &data)
}

/// Check if a card is expired and update its status. Returns true if status changed.
fn check_lazy_expiry(card: &mut Value) {
    let status = card.get("status").and_then(|s| s.as_str()).unwrap_or("");
    if status == "expired" || status == "redeemed" {
        return;
    }
    if let Some(expires_at) = card.get("expiresAt").and_then(|e| e.as_str()) {
        if !expires_at.is_empty() {
            if let Ok(exp_ts) = expires_at.parse::<u64>() {
                let now = h_now();
                if now > exp_ts {
                    card["status"] = json!("expired");
                    card["expiredAt"] = json!(now.to_string());
                    // Persist the status change so we don't re-process on the next read.
                    let id = card.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if !id.is_empty() {
                        h_storage_set("cards", card);
                    }
                }
            }
        }
    }
}

/// Generate a card ID, raw token, and token hash.
/// raw_token is 32 random bytes URL-safe base64 encoded; token_hash is its
/// SHA-256 hex digest; card_id is "gc_" + first 16 hex chars of the hash.
fn generate_tokens() -> (String, String, String) {
    let raw_token = generate_magic_token();
    let token_hash = sha256_hex(raw_token.as_bytes());
    let card_id = format!("gc_{}", &token_hash[..16]);
    (card_id, raw_token, token_hash)
}

// ---------------------------------------------------------------------------
// Export implementations
// ---------------------------------------------------------------------------

struct Component;

impl Guest for Component {
    // --- Authenticated API ---

    fn create_card(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let amount = match req.get("amount").and_then(|a| a.as_u64()) {
            Some(a) if a > 0 && a <= u64::MAX / 1000 => a,
            _ => return err("Amount must be a positive integer within the Lightning amount limit"),
        };

        let wallet_id = match resolve_user_wallet(&req) {
            Ok(wallet_id) => wallet_id,
            Err(message) => return err(&message),
        };

        let base_url = creation_base_url(&req);

        let (card_id, raw_token, token_hash) = generate_tokens();
        let now = h_now();

        let redemption_url = format!("{}/ext/giftcardswasm/redeem/{}", base_url, raw_token);
        let lnurl_url = format!("{}/api/v1/ext/giftcardswasm/lnurl/{}", base_url, token_hash);

        let fee_mode = req.get("feeMode").and_then(|v| v.as_str()).unwrap_or("default").to_string();
        let fee_percent = req.get("feePercent")
            .map(|v| {
                if v.is_string() {
                    v.as_str().unwrap_or("0").to_string()
                } else if v.is_f64() {
                    format!("{}", v.as_f64().unwrap_or(0.0))
                } else if v.is_i64() {
                    format!("{}", v.as_i64().unwrap_or(0))
                } else if v.is_u64() {
                    format!("{}", v.as_u64().unwrap_or(0))
                } else {
                    "0".to_string()
                }
            })
            .unwrap_or_else(|| "0".to_string());
        let fee_sats = req.get("feeSats").and_then(|v| v.as_u64()).unwrap_or(0);

        let mut card = json!({
            "id": token_hash,
            "cardId": card_id,
            "walletId": wallet_id,
            "amount": amount,
            "tokenHash": token_hash,
            "rawToken": raw_token,
            "redemptionUrl": redemption_url,
            "status": "active",
            "recipientName": req.get("recipientName").and_then(|v| v.as_str()).unwrap_or(""),
            "senderName": req.get("senderName").and_then(|v| v.as_str()).unwrap_or(""),
            "message": req.get("message").and_then(|v| v.as_str()).unwrap_or(""),
            "recipientEmail": req.get("recipientEmail").and_then(|v| v.as_str()).unwrap_or(""),
            "emailStatus": "not_sent",
            "createdAt": now.to_string(),
            "expiresAt": req.get("expiresAt").and_then(|v| v.as_str()).unwrap_or(""),
            "redeemedAt": "",
            "expiredAt": "",
            "baseUrl": base_url,
            "feeMode": fee_mode,
            "feePercent": fee_percent,
            "feeSats": fee_sats,
        });

        // Split the design object into separate qr_config and text_config JSON
        // columns (mirrors the Python GiftCard.qr_config / text_config fields).
        // Also keep a designJson field for backward compatibility with readers.
        if let Some(design) = req.get("design") {
            if !design.is_null() {
                let qr_config = json!({
                    "qr_x_frac": design.get("qr_x_frac").and_then(|v| v.as_f64()).unwrap_or(0.1),
                    "qr_y_frac": design.get("qr_y_frac").and_then(|v| v.as_f64()).unwrap_or(0.7),
                    "qr_size": design.get("qr_size").and_then(|v| v.as_u64()).unwrap_or(200),
                });
                let text_config = json!({
                    "text_x_frac": design.get("text_x_frac").and_then(|v| v.as_f64()).unwrap_or(0.1),
                    "text_y_frac": design.get("text_y_frac").and_then(|v| v.as_f64()).unwrap_or(0.1),
                    "font_family": design.get("font_family").and_then(|v| v.as_str()).unwrap_or("DejaVuSans"),
                    "font_size": design.get("font_size").and_then(|v| v.as_u64()).unwrap_or(24),
                    "font_color": design.get("font_color").and_then(|v| v.as_str()).unwrap_or("#000000"),
                    "bg_color": design.get("bg_color").and_then(|v| v.as_str()).unwrap_or(""),
                    "text_align": design.get("text_align").and_then(|v| v.as_str()).unwrap_or("left"),
                    "show_amount": design.get("show_amount").and_then(|v| v.as_bool()).unwrap_or(true),
                    "show_recipient": design.get("show_recipient").and_then(|v| v.as_bool()).unwrap_or(true),
                    "show_message": design.get("show_message").and_then(|v| v.as_bool()).unwrap_or(true),
                });
                card["qrConfig"] = Value::String(serde_json::to_string(&qr_config).unwrap_or_default());
                card["textConfig"] = Value::String(serde_json::to_string(&text_config).unwrap_or_default());
                card["templateName"] = json!(design.get("templateName").and_then(|v| v.as_str()).unwrap_or(design.get("template_name").and_then(|v| v.as_str()).unwrap_or("portrait")));
                card["templateAssetId"] = json!(design.get("templateAssetId").and_then(|v| v.as_str()).unwrap_or(design.get("template_asset_id").and_then(|v| v.as_str()).unwrap_or("")));
                card["designJson"] = Value::String(serde_json::to_string(design).unwrap_or_default());
            }
        }

        if !h_storage_set("cards", &card) {
            return err("Failed to store gift card");
        }

        ok(json!({
            "cardId": card_id,
            "rawToken": raw_token,
            "tokenHash": token_hash,
            "redemptionUrl": redemption_url,
            "lnurlUrl": lnurl_url,
        }))
    }

    fn get_wallets(_payload: String) -> String {
        let wallets: Vec<Value> = h_list_wallets()
            .into_iter()
            .map(|(id, name)| json!({"id": id, "name": name}))
            .collect();
        ok(json!(wallets))
    }

    fn get_settings(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let wallet_id = match resolve_user_wallet(&req) {
            Ok(wallet_id) => wallet_id,
            Err(message) => return err(&message),
        };

        match h_storage_get("settings", &wallet_id) {
            Some(row) => ok(json!({
                "darkMode": row.get("darkMode").and_then(|v| v.as_str()).unwrap_or(""),
            })),
            None => ok(json!({"darkMode": ""})),
        }
    }

    fn set_settings(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let wallet_id = match resolve_user_wallet(&req) {
            Ok(wallet_id) => wallet_id,
            Err(message) => return err(&message),
        };

        let dark_mode = match req.get("darkMode") {
            Some(Value::Bool(b)) => b.to_string(),
            Some(Value::String(s)) if s == "true" || s == "false" => s.clone(),
            _ => return err("darkMode must be true or false"),
        };

        let row = json!({
            "id": wallet_id,
            "walletId": wallet_id,
            "darkMode": dark_mode,
        });
        if !h_storage_set("settings", &row) {
            return err("Failed to save settings");
        }
        ok(json!({"ok": true}))
    }

    fn get_cards(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let wallet_id = match resolve_user_wallet(&req) {
            Ok(wallet_id) => wallet_id,
            Err(message) => return err(&message),
        };

        let mut filters = json!({"walletId": wallet_id});
        if let Some(status) = req.get("status").and_then(|s| s.as_str()) {
            if !status.is_empty() {
                filters["status"] = json!(status);
            }
        }

        let (rows, _total) = h_storage_get_paginated("cards", &filters, 1000, 0);

        let summaries: Vec<Value> = rows
            .iter()
            .map(|card| {
                let mut c = card.clone();
                check_lazy_expiry(&mut c);
                let base_url = c.get("baseUrl").and_then(|v| v.as_str()).unwrap_or("");
                let raw_token = c.get("rawToken").and_then(|v| v.as_str()).unwrap_or("");
                let redemption_url = if !raw_token.is_empty() {
                    format!("{}/ext/giftcardswasm/redeem/{}", base_url, raw_token)
                } else {
                    String::new()
                };
                json!({
                    "id": c.get("cardId").and_then(|v| v.as_str()).unwrap_or(""),
                    "amount": c.get("amount").and_then(|v| v.as_u64()).unwrap_or(0),
                    "status": c.get("status").and_then(|v| v.as_str()).unwrap_or("active"),
                    "recipientName": c.get("recipientName").and_then(|v| v.as_str()).unwrap_or(""),
                    "senderName": c.get("senderName").and_then(|v| v.as_str()).unwrap_or(""),
                    "message": c.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                    "recipientEmail": c.get("recipientEmail").and_then(|v| v.as_str()).unwrap_or(""),
                    "emailStatus": c.get("emailStatus").and_then(|v| v.as_str()).unwrap_or("not_sent"),
                    "createdAt": c.get("createdAt").and_then(|v| v.as_str()).unwrap_or(""),
                    "expiresAt": c.get("expiresAt").and_then(|v| v.as_str()).unwrap_or(""),
                    "redeemedAt": c.get("redeemedAt").and_then(|v| v.as_str()).unwrap_or(""),
                    "expiredAt": c.get("expiredAt").and_then(|v| v.as_str()).unwrap_or(""),
                    "templateName": c.get("templateName").and_then(|v| v.as_str()).unwrap_or(""),
                    "templateAssetId": c.get("templateAssetId").and_then(|v| v.as_str()).unwrap_or(""),
                    "redemptionUrl": redemption_url,
                    "feeMode": c.get("feeMode").and_then(|v| v.as_str()).unwrap_or("default"),
                    "feePercent": c.get("feePercent").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0),
                    "feeSats": c.get("feeSats").and_then(|v| v.as_u64()).unwrap_or(0),
                })
            })
            .collect();

        ok(json!(summaries))
    }

    fn get_card(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let card_id = match req.get("cardId").and_then(|c| c.as_str()) {
            Some(c) if !c.is_empty() => c,
            _ => return err("cardId is required"),
        };

        // card_id is the display ID (gc_xxx), but storage id is token_hash.
        // Verify that the card belongs to one of the current user's wallets.
        let mut card = match owned_card(card_id) {
            Some(card) => card,
            None => return err("Gift card not found"),
        };

        check_lazy_expiry(&mut card);

        let include_link = req.get("includeLink").and_then(|v| {
            v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))
        }).unwrap_or(false);
        let redemption_url = if include_link {
            let base_url = card.get("baseUrl").and_then(|v| v.as_str()).unwrap_or("");
            let raw_token = card.get("rawToken").and_then(|v| v.as_str()).unwrap_or("");
            format!("{}/ext/giftcardswasm/redeem/{}", base_url, raw_token)
        } else {
            String::new()
        };

        // Reconstruct the design object from the split qr_config / text_config
        // columns, falling back to the legacy designJson field.
        let design: Option<Value> = {
            let qr = card.get("qrConfig").and_then(|v| v.as_str()).and_then(|s| serde_json::from_str(s).ok());
            let txt = card.get("textConfig").and_then(|v| v.as_str()).and_then(|s| serde_json::from_str(s).ok());
            if qr.is_some() || txt.is_some() {
                let qr = qr.unwrap_or(json!({}));
                let txt = txt.unwrap_or(json!({}));
                Some(json!({
                    "templateName": card.get("templateName").and_then(|v| v.as_str()).unwrap_or("portrait"),
                    "templateAssetId": card.get("templateAssetId").and_then(|v| v.as_str()).unwrap_or(""),
                    "qrXFrac": qr.get("qr_x_frac").and_then(|v| v.as_f64()).unwrap_or(0.1),
                    "qrYFrac": qr.get("qr_y_frac").and_then(|v| v.as_f64()).unwrap_or(0.7),
                    "qrSize": qr.get("qr_size").and_then(|v| v.as_u64()).unwrap_or(200),
                    "textXFrac": txt.get("text_x_frac").and_then(|v| v.as_f64()).unwrap_or(0.1),
                    "textYFrac": txt.get("text_y_frac").and_then(|v| v.as_f64()).unwrap_or(0.1),
                    "fontFamily": txt.get("font_family").and_then(|v| v.as_str()).unwrap_or("DejaVuSans"),
                    "fontSize": txt.get("font_size").and_then(|v| v.as_u64()).unwrap_or(24),
                    "fontColor": txt.get("font_color").and_then(|v| v.as_str()).unwrap_or("#000000"),
                    "bgColor": txt.get("bg_color").and_then(|v| v.as_str()).unwrap_or(""),
                    "textAlign": txt.get("text_align").and_then(|v| v.as_str()).unwrap_or("left"),
                    "showAmount": txt.get("show_amount").and_then(|v| v.as_bool()).unwrap_or(true),
                    "showRecipient": txt.get("show_recipient").and_then(|v| v.as_bool()).unwrap_or(true),
                    "showMessage": txt.get("show_message").and_then(|v| v.as_bool()).unwrap_or(true),
                }))
            } else {
                card.get("designJson").and_then(|v| v.as_str()).and_then(|s| serde_json::from_str(s).ok())
            }
        };

        ok(json!({
            "cardId": card.get("cardId").and_then(|v| v.as_str()).unwrap_or(""),
            "amount": card.get("amount").and_then(|v| v.as_u64()).unwrap_or(0),
            "status": card.get("status").and_then(|v| v.as_str()).unwrap_or("active"),
            "recipientName": card.get("recipientName").and_then(|v| v.as_str()).unwrap_or(""),
            "senderName": card.get("senderName").and_then(|v| v.as_str()).unwrap_or(""),
            "recipientEmail": card.get("recipientEmail").and_then(|v| v.as_str()).unwrap_or(""),
            "message": card.get("message").and_then(|v| v.as_str()).unwrap_or(""),
            "createdAt": card.get("createdAt").and_then(|v| v.as_str()).unwrap_or(""),
            "expiresAt": card.get("expiresAt").and_then(|v| v.as_str()).unwrap_or(""),
            "redeemedAt": card.get("redeemedAt").and_then(|v| v.as_str()).unwrap_or(""),
            "emailStatus": card.get("emailStatus").and_then(|v| v.as_str()).unwrap_or("not_sent"),
            "tokenHash": card.get("tokenHash").and_then(|v| v.as_str()).unwrap_or(""),
            "redemptionUrl": redemption_url,
            "design": design,
            "feeMode": card.get("feeMode").and_then(|v| v.as_str()).unwrap_or("default"),
            "feePercent": card.get("feePercent").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0),
            "feeSats": card.get("feeSats").and_then(|v| v.as_u64()).unwrap_or(0),
        }))
    }

    fn update_card(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let card_id = match req.get("cardId").and_then(|c| c.as_str()) {
            Some(c) if !c.is_empty() => c,
            _ => return err("cardId is required"),
        };

        let mut card = match owned_card(card_id) {
            Some(card) => card,
            None => return err("Gift card not found"),
        };

        // Update metadata fields
        for field in &["recipientName", "senderName", "message", "recipientEmail"] {
            if let Some(val) = req.get(*field) {
                if !val.is_null() {
                    card[*field] = val.clone();
                }
            }
        }

        // Update design — split into qr_config and text_config JSON columns.
        if req.get("clearDesign").and_then(|v| v.as_bool()).unwrap_or(false) {
            card["designJson"] = json!(null);
            card["qrConfig"] = json!(null);
            card["textConfig"] = json!(null);
            card["templateName"] = json!("");
            card["templateAssetId"] = json!("");
        } else if let Some(design) = req.get("design") {
            if !design.is_null() {
                let qr_config = json!({
                    "qr_x_frac": design.get("qr_x_frac").and_then(|v| v.as_f64()).unwrap_or(0.1),
                    "qr_y_frac": design.get("qr_y_frac").and_then(|v| v.as_f64()).unwrap_or(0.7),
                    "qr_size": design.get("qr_size").and_then(|v| v.as_u64()).unwrap_or(200),
                });
                let text_config = json!({
                    "text_x_frac": design.get("text_x_frac").and_then(|v| v.as_f64()).unwrap_or(0.1),
                    "text_y_frac": design.get("text_y_frac").and_then(|v| v.as_f64()).unwrap_or(0.1),
                    "font_family": design.get("font_family").and_then(|v| v.as_str()).unwrap_or("DejaVuSans"),
                    "font_size": design.get("font_size").and_then(|v| v.as_u64()).unwrap_or(24),
                    "font_color": design.get("font_color").and_then(|v| v.as_str()).unwrap_or("#000000"),
                    "bg_color": design.get("bg_color").and_then(|v| v.as_str()).unwrap_or(""),
                    "text_align": design.get("text_align").and_then(|v| v.as_str()).unwrap_or("left"),
                    "show_amount": design.get("show_amount").and_then(|v| v.as_bool()).unwrap_or(true),
                    "show_recipient": design.get("show_recipient").and_then(|v| v.as_bool()).unwrap_or(true),
                    "show_message": design.get("show_message").and_then(|v| v.as_bool()).unwrap_or(true),
                });
                card["qrConfig"] = Value::String(serde_json::to_string(&qr_config).unwrap_or_default());
                card["textConfig"] = Value::String(serde_json::to_string(&text_config).unwrap_or_default());
                card["templateName"] = json!(design.get("templateName").and_then(|v| v.as_str()).unwrap_or(design.get("template_name").and_then(|v| v.as_str()).unwrap_or("portrait")));
                card["templateAssetId"] = json!(design.get("templateAssetId").and_then(|v| v.as_str()).unwrap_or(design.get("template_asset_id").and_then(|v| v.as_str()).unwrap_or("")));
                card["designJson"] = Value::String(serde_json::to_string(design).unwrap_or_default());
            }
        }

        if !h_storage_set("cards", &card) {
            return err("Failed to update gift card");
        }

        ok(json!({"status": "updated"}))
    }

    fn delete_card(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let card_id = match req.get("cardId").and_then(|c| c.as_str()) {
            Some(c) if !c.is_empty() => c,
            _ => return err("cardId is required"),
        };

        let card = match owned_card(card_id) {
            Some(card) => card,
            None => return err("Gift card not found"),
        };

        let id = card.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        h_storage_delete("cards", &id);

        ok(json!({"status": "deleted"}))
    }

    fn bulk_create(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let wallet_id = match resolve_user_wallet(&req) {
            Ok(wallet_id) => wallet_id,
            Err(message) => return err(&message),
        };

        let base_url = creation_base_url(&req);

        let mut card_ids: Vec<String> = Vec::new();

        // CSV mode (rows provided) or same-amount mode
        if let Some(rows) = req.get("rows").and_then(|r| r.as_array()) {
            for row in rows {
                let amount = row.get("amountSats").and_then(|a| a.as_u64()).unwrap_or(0);
                if amount == 0 {
                    continue;
                }
                let mut create_req = json!({
                    "amount": amount,
                    "walletId": wallet_id,
                    "baseUrl": base_url,
                    "recipientName": row.get("recipientName").and_then(|v| v.as_str()).unwrap_or(""),
                    "senderName": row.get("senderName").and_then(|v| v.as_str()).unwrap_or(""),
                    "message": row.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                    "recipientEmail": row.get("recipientEmail").and_then(|v| v.as_str()).unwrap_or(""),
                    "feeMode": req.get("feeMode").and_then(|v| v.as_str()).unwrap_or("default"),
                    "feePercent": req.get("feePercent").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    "feeSats": req.get("feeSats").and_then(|v| v.as_u64()).unwrap_or(0),
                });
                if let Some(design) = req.get("design") {
                    if !design.is_null() {
                        create_req["design"] = design.clone();
                    }
                }
                let result = Self::create_card(create_req.to_string());
                if let Ok(resp) = serde_json::from_str::<Value>(&result) {
                    if let Some(id) = resp.get("cardId").and_then(|v| v.as_str()) {
                        card_ids.push(id.to_string());
                    }
                }
            }
        } else {
            let count = req.get("count").and_then(|c| c.as_u64()).unwrap_or(1) as usize;
            let amount = req.get("amount").and_then(|a| a.as_u64()).unwrap_or(0);
            if amount == 0 {
                return err("Amount must be positive");
            }

            let template = json!({
                "amount": amount,
                "walletId": wallet_id,
                "baseUrl": base_url,
                "recipientName": req.get("recipientName").and_then(|v| v.as_str()).unwrap_or(""),
                "senderName": req.get("senderName").and_then(|v| v.as_str()).unwrap_or(""),
                "message": req.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                "recipientEmail": req.get("recipientEmail").and_then(|v| v.as_str()).unwrap_or(""),
                "expiresAt": req.get("expiresAt").and_then(|v| v.as_str()).unwrap_or(""),
                "design": req.get("design"),
                "feeMode": req.get("feeMode").and_then(|v| v.as_str()).unwrap_or("default"),
                "feePercent": req.get("feePercent").and_then(|v| v.as_f64()).unwrap_or(0.0),
                "feeSats": req.get("feeSats").and_then(|v| v.as_u64()).unwrap_or(0),
            });

            for _ in 0..count {
                let result = Self::create_card(template.to_string());
                if let Ok(resp) = serde_json::from_str::<Value>(&result) {
                    if let Some(id) = resp.get("cardId").and_then(|v| v.as_str()) {
                        card_ids.push(id.to_string());
                    }
                }
            }
        }

        ok(json!({
            "created": card_ids.len(),
            "cardIds": card_ids,
        }))
    }

    // --- Public API ---

    fn get_public_card(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let token_hash = match req.get("tokenHash").and_then(|t| t.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => return err("tokenHash is required"),
        };

        let mut card = match h_storage_get_public("cards", token_hash) {
            Some(c) => c,
            None => return err("Gift card not found"),
        };

        check_lazy_expiry(&mut card);

        let status = card.get("status").and_then(|s| s.as_str()).unwrap_or("active");
        let has_design = card.get("designJson").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        let template_name = card.get("templateName").and_then(|v| v.as_str()).unwrap_or("");
        let template_asset_id = card.get("templateAssetId").and_then(|v| v.as_str()).unwrap_or("");
        // hasDesign is true if there's a non-empty template name or designJson
        let has_design = has_design || !template_name.is_empty();

        // Parse qrConfig and textConfig (stored as JSON strings) so the
        // redeem page can position the QR code and text overlay at the
        // same coordinates the user chose in the card designer.
        let qr_config = card.get("qrConfig").and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str::<Value>(s).ok());
        let text_config = card.get("textConfig").and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str::<Value>(s).ok());

        ok(json!({
            "status": status,
            "amount": card.get("amount").and_then(|v| v.as_u64()).unwrap_or(0),
            "senderName": card.get("senderName").and_then(|v| v.as_str()).unwrap_or(""),
            "recipientName": card.get("recipientName").and_then(|v| v.as_str()).unwrap_or(""),
            "message": card.get("message").and_then(|v| v.as_str()).unwrap_or(""),
            "expiresAt": card.get("expiresAt").and_then(|v| v.as_str()).unwrap_or(""),
            "expiredAt": card.get("expiredAt").and_then(|v| v.as_str()).unwrap_or(""),
            "hasDesign": has_design,
            "templateName": template_name,
            "templateAssetId": template_asset_id,
            "qrConfig": qr_config,
            "textConfig": text_config,
        }))
    }

    fn lnurl_params(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let token_hash = match req.get("tokenHash").and_then(|t| t.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => return err("tokenHash is required"),
        };

        let mut card = match h_storage_get("cards", token_hash) {
            Some(c) => c,
            None => return err("Gift card not found"),
        };

        check_lazy_expiry(&mut card);

        let status = card.get("status").and_then(|s| s.as_str()).unwrap_or("active");

        // Never automatically unlock a card after a payment attempt. A host
        // crash can happen after the invoice is paid but before the card is
        // marked redeemed; unlocking here would permit a second payment.
        if status != "active" {
            return ok(json!({
                "status": "ERROR",
                "reason": "Gift card is not available for redemption",
            }));
        }

        let amount = card.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
        // Prefer the public base URL injected by the host (from X-Forwarded-*
        // headers) so the callback URL is reachable by external wallets.
        // Fall back only to a previously validated stored URL; never trust a
        // caller-supplied baseUrl because it can turn the LNURL into a link
        // to an attacker-controlled host.
        let injected_base_url = trusted_base_url(&req);
        let stored_base_url = card
            .get("baseUrl")
            .and_then(|b| b.as_str())
            .map(sanitize_base_url)
            .unwrap_or_default();
        let base_url = if injected_base_url.is_empty() {
            stored_base_url
        } else {
            injected_base_url
        };
        let card_id = card.get("cardId").and_then(|v| v.as_str()).unwrap_or("");

        let callback_url = format!("{}/api/v1/ext/giftcardswasm/lnurl/callback", base_url);

        ok(json!({
            "tag": "withdrawRequest",
            "callback": callback_url,
            "k1": token_hash,
            "defaultDescription": format!("Gift card {}", &card_id[..card_id.len().min(8)]),
            "minWithdrawable": amount * 1000,
            "maxWithdrawable": amount * 1000,
        }))
    }

    fn lnurl_callback(payload: String) -> String {
        let req: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => return err(&format!("Invalid request: {e}")),
        };

        let pr = match req.get("pr").and_then(|p| p.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return ok(json!({"status": "ERROR", "reason": "Payment request is required"})),
        };

        let k1 = match req.get("k1").and_then(|k| k.as_str()) {
            Some(k) if !k.is_empty() => k,
            _ => return ok(json!({"status": "ERROR", "reason": "Redemption token is required"})),
        };

        // Look up card by token_hash (k1 = token_hash = storage id)
        let mut card = match h_storage_get("cards", k1) {
            Some(c) => c,
            None => return ok(json!({"status": "ERROR", "reason": "Gift card not found"})),
        };

        check_lazy_expiry(&mut card);

        // Do not unlock a card that is already redeeming. Without an atomic
        // storage compare-and-set, unlocking after a crash can double-pay.
        let status = card.get("status").and_then(|s| s.as_str()).unwrap_or("active");
        if status != "active" {
            return ok(json!({"status": "ERROR", "reason": "Gift card is not available for redemption"}));
        }
        // The storage API has no conditional update, so this lock is not
        // atomic across concurrent WASM invocations. The host should provide
        // compare-and-set semantics before this endpoint is used in production.
        // Carry over any expiry side-effects from check_lazy_expiry above.
        card["status"] = json!("redeeming");
        let id = card.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if !h_storage_set("cards", &card) {
            return ok(json!({"status": "ERROR", "reason": "Failed to lock card"}));
        }
        // Pay the invoice from the issuer's wallet
        let wallet_id = card.get("walletId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let amount = card.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);

        let pay_result = h_pay_invoice(&wallet_id, pr, amount);

        let success = pay_result.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
        let pending = pay_result.get("pending").and_then(|p| p.as_bool()).unwrap_or(false);

        if success || pending {
            // Mark as redeemed
            card["status"] = json!("redeemed");
            card["redeemedAt"] = json!(h_now().to_string());
            h_storage_set("cards", &card);
            ok(json!({"status": "OK"}))
        } else {
            // Reset to active
            card["status"] = json!("active");
            h_storage_set("cards", &card);
            let error_msg = pay_result.get("error").and_then(|e| e.as_str()).unwrap_or("Payment failed");
            h_log("error", &format!("Redemption failed for card {}: {}", &id[..id.len().min(8)], error_msg));
            ok(json!({"status": "ERROR", "reason": "Redemption failed. Please try again."}))
        }
    }

    // --- Event handler ---

    fn on_invoice_paid(_payload: String) -> String {
        // Invoice payment events are handled by the LNURL callback directly.
        // This event handler is a no-op but required for the event export.
        ok(json!({"ok": true}))
    }
}

export!(Component);
