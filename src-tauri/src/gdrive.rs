//! Google Drive integration module
//! Handles OAuth2 authentication and Google Drive API operations

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::database::get_app_data_dir;

// Backend auth server URL (handles OAuth securely)
// This keeps client_id and client_secret on the server
const AUTH_SERVER_URL: &str = "https://streamvault-backend-server.onrender.com";

// Google Drive API
const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";

/// Stored OAuth tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub token_type: String,
}

/// Google Drive account info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveAccountInfo {
    pub email: String,
    pub display_name: Option<String>,
    pub photo_url: Option<String>,
    pub storage_used: Option<i64>,
    pub storage_limit: Option<i64>,
}

/// Google Drive file/folder item
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveItem {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    #[serde(default)]
    pub size: Option<String>,
    pub modified_time: Option<String>,
    pub parents: Option<Vec<String>>,
    #[serde(default)]
    pub web_content_link: Option<String>,
}

/// Response from Drive API files.list
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveListResponse {
    pub files: Vec<DriveItem>,
    pub next_page_token: Option<String>,
}

/// Response from Drive API changes.list
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveChangesResponse {
    pub changes: Vec<DriveChange>,
    pub new_start_page_token: Option<String>,
    pub next_page_token: Option<String>,
}

/// A single change from the Changes API
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveChange {
    pub kind: Option<String>,
    pub removed: Option<bool>,
    pub file: Option<DriveItem>,
    pub file_id: Option<String>,
    pub change_type: Option<String>,
}

/// Google Drive client state
pub struct GoogleDriveClient {
    tokens: Arc<Mutex<Option<GoogleTokens>>>,
    http_client: reqwest::Client,
}

impl GoogleDriveClient {
    pub fn new() -> Self {
        let tokens = load_tokens().ok();
        Self {
            tokens: Arc::new(Mutex::new(tokens)),
            http_client: reqwest::Client::new(),
        }
    }

    /// Check if user is authenticated
    pub fn is_authenticated(&self) -> bool {
        self.tokens.lock().unwrap().is_some()
    }

    /// Get the current access token, refreshing if needed
    pub async fn get_access_token(&self) -> Result<String, String> {
        let tokens = self.tokens.lock().unwrap().clone();

        match tokens {
            Some(t) => {
                // Check if token is expired
                if let Some(expires_at) = t.expires_at {
                    let now = chrono::Utc::now().timestamp();
                    if now >= expires_at - 60 {
                        // Token expired or about to expire, refresh it
                        if let Some(refresh_token) = &t.refresh_token {
                            return self.refresh_access_token(refresh_token).await;
                        }
                        return Err("Token expired and no refresh token available".to_string());
                    }
                }
                Ok(t.access_token)
            }
            None => Err("Not authenticated".to_string()),
        }
    }

    /// Refresh the access token via backend proxy
    async fn refresh_access_token(&self, refresh_token: &str) -> Result<String, String> {
        let response = self.http_client
            .post(format!("{}/auth/refresh", AUTH_SERVER_URL))
            .json(&serde_json::json!({
                "refresh_token": refresh_token
            }))
            .send()
            .await
            .map_err(|e| format!("Failed to refresh token: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Token refresh failed: {}", error_text));
        }

        let token_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse token response: {}", e))?;

        let access_token = token_response["access_token"]
            .as_str()
            .ok_or("Missing access_token in response")?
            .to_string();

        let expires_in = token_response["expires_in"].as_i64().unwrap_or(3600);
        let expires_at = chrono::Utc::now().timestamp() + expires_in;

        // Update stored tokens
        let mut tokens = self.tokens.lock().unwrap();
        if let Some(ref mut t) = *tokens {
            t.access_token = access_token.clone();
            t.expires_at = Some(expires_at);
            save_tokens(t).ok();
        }

        Ok(access_token)
    }

    /// Store tokens after successful authentication
    pub fn store_tokens(&self, tokens: GoogleTokens) -> Result<(), String> {
        save_tokens(&tokens)?;
        *self.tokens.lock().unwrap() = Some(tokens);
        Ok(())
    }

    /// Clear stored tokens (logout)
    pub fn clear_tokens(&self) -> Result<(), String> {
        *self.tokens.lock().unwrap() = None;
        let path = get_tokens_path();
        if path.exists() {
            fs::remove_file(path).map_err(|e| format!("Failed to remove tokens: {}", e))?;
        }
        Ok(())
    }

    /// List files in a folder
    pub async fn list_files(
        &self,
        folder_id: Option<&str>,
        page_token: Option<&str>,
    ) -> Result<DriveListResponse, String> {
        let access_token = self.get_access_token().await?;

        let parent = folder_id.unwrap_or("root");
        let query = format!("'{}' in parents and trashed = false", parent);

        let mut url = format!(
            "{}/files?q={}&fields=files(id,name,mimeType,size,modifiedTime,parents,webContentLink),nextPageToken&pageSize=100&orderBy=name",
            DRIVE_API_BASE,
            urlencoding::encode(&query)
        );

        if let Some(token) = page_token {
            url.push_str(&format!("&pageToken={}", token));
        }

        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| format!("Failed to list files: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Drive API error: {}", error_text));
        }

        response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))
    }

    /// List only folders
    pub async fn list_folders(&self, parent_id: Option<&str>) -> Result<Vec<DriveItem>, String> {
        let access_token = self.get_access_token().await?;

        let parent = parent_id.unwrap_or("root");
        let query = format!(
            "'{}' in parents and mimeType = 'application/vnd.google-apps.folder' and trashed = false",
            parent
        );

        let url = format!(
            "{}/files?q={}&fields=files(id,name,mimeType,modifiedTime,parents)&pageSize=100&orderBy=name",
            DRIVE_API_BASE,
            urlencoding::encode(&query)
        );

        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| format!("Failed to list folders: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Drive API error: {}", error_text));
        }

        let result: DriveListResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(result.files)
    }

    /// List video files in a folder (recursive option)
    pub async fn list_video_files(
        &self,
        folder_id: &str,
        recursive: bool,
    ) -> Result<Vec<DriveItem>, String> {
        let access_token = self.get_access_token().await?;

        let video_mimes = [
            "video/mp4",
            "video/x-matroska",
            "video/avi",
            "video/quicktime",
            "video/webm",
            "video/x-m4v",
            "video/x-ms-wmv",
            "video/x-flv",
            "video/mp2t",
        ];

        let mime_conditions: Vec<String> = video_mimes
            .iter()
            .map(|m| format!("mimeType = '{}'", m))
            .collect();

        let query = format!(
            "'{}' in parents and ({}) and trashed = false",
            folder_id,
            mime_conditions.join(" or ")
        );

        let mut all_files = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut url = format!(
                "{}/files?q={}&fields=files(id,name,mimeType,size,modifiedTime,parents,webContentLink),nextPageToken&pageSize=100",
                DRIVE_API_BASE,
                urlencoding::encode(&query)
            );

            if let Some(ref token) = page_token {
                url.push_str(&format!("&pageToken={}", token));
            }

            let response = self.http_client
                .get(&url)
                .header("Authorization", format!("Bearer {}", access_token))
                .send()
                .await
                .map_err(|e| format!("Failed to list video files: {}", e))?;

            if !response.status().is_success() {
                let error_text = response.text().await.unwrap_or_default();
                return Err(format!("Drive API error: {}", error_text));
            }

            let result: DriveListResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse response: {}", e))?;

            all_files.extend(result.files);

            if let Some(next_token) = result.next_page_token {
                page_token = Some(next_token);
            } else {
                break;
            }
        }

        // If recursive, also scan subfolders
        if recursive {
            let subfolders = self.list_folders(Some(folder_id)).await?;
            for folder in subfolders {
                let subfolder_files = Box::pin(self.list_video_files(&folder.id, true)).await?;
                all_files.extend(subfolder_files);
            }
        }

        Ok(all_files)
    }

    /// Get a streaming URL for a file (with auth header)
    pub async fn get_stream_url(&self, file_id: &str) -> Result<(String, String), String> {
        let access_token = self.get_access_token().await?;
        let url = format!("{}/files/{}?alt=media", DRIVE_API_BASE, file_id);
        Ok((url, access_token))
    }

    /// Get file metadata
    pub async fn get_file_metadata(&self, file_id: &str) -> Result<DriveItem, String> {
        let access_token = self.get_access_token().await?;

        let url = format!(
            "{}/files/{}?fields=id,name,mimeType,size,modifiedTime,parents,webContentLink",
            DRIVE_API_BASE,
            file_id
        );

        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| format!("Failed to get file metadata: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Drive API error: {}", error_text));
        }

        response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))
    }

    /// Delete a file from Google Drive
    pub async fn delete_file(&self, file_id: &str) -> Result<(), String> {
        let access_token = self.get_access_token().await?;

        let url = format!("{}/files/{}", DRIVE_API_BASE, file_id);

        let response = self.http_client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| format!("Failed to delete file: {}", e))?;

        // Google Drive API returns 204 No Content on successful deletion
        if response.status().is_success() || response.status().as_u16() == 204 {
            println!("[GDRIVE] Successfully deleted file: {}", file_id);
            Ok(())
        } else {
            let error_text = response.text().await.unwrap_or_default();
            Err(format!("Drive API delete error: {}", error_text))
        }
    }

    /// Get account info
    pub async fn get_account_info(&self) -> Result<DriveAccountInfo, String> {
        let access_token = self.get_access_token().await?;

        // Get user info
        let user_url = "https://www.googleapis.com/oauth2/v2/userinfo";
        let user_response = self.http_client
            .get(user_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| format!("Failed to get user info: {}", e))?;

        if !user_response.status().is_success() {
            let error_text = user_response.text().await.unwrap_or_default();
            return Err(format!("User info API error: {}", error_text));
        }

        let user_info: serde_json::Value = user_response
            .json()
            .await
            .map_err(|e| format!("Failed to parse user info: {}", e))?;

        // Get storage quota
        let quota_url = format!("{}/about?fields=storageQuota,user", DRIVE_API_BASE);
        let quota_response = self.http_client
            .get(&quota_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .ok();

        let (storage_used, storage_limit) = if let Some(resp) = quota_response {
            if let Ok(quota_info) = resp.json::<serde_json::Value>().await {
                let used = quota_info["storageQuota"]["usage"]
                    .as_str()
                    .and_then(|s| s.parse().ok());
                let limit = quota_info["storageQuota"]["limit"]
                    .as_str()
                    .and_then(|s| s.parse().ok());
                (used, limit)
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        Ok(DriveAccountInfo {
            email: user_info["email"].as_str().unwrap_or("").to_string(),
            display_name: user_info["name"].as_str().map(String::from),
            photo_url: user_info["picture"].as_str().map(String::from),
            storage_used,
            storage_limit,
        })
    }

    // ==================== Changes API (Efficient Delta Sync) ====================

    /// Get the start page token for tracking changes
    /// Call this once when setting up change tracking
    pub async fn get_changes_start_token(&self) -> Result<String, String> {
        let access_token = self.get_access_token().await?;

        let url = format!("{}/changes/startPageToken", DRIVE_API_BASE);

        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| format!("Failed to get start page token: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Drive API error: {}", error_text));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        result["startPageToken"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| "Missing startPageToken in response".to_string())
    }

    /// Get changes since the given page token
    /// Returns new/modified files and a new token for the next check
    pub async fn get_changes(&self, page_token: &str) -> Result<DriveChangesResponse, String> {
        let access_token = self.get_access_token().await?;

        let url = format!(
            "{}/changes?pageToken={}&fields=changes(fileId,removed,file(id,name,mimeType,size,modifiedTime,parents)),newStartPageToken,nextPageToken&pageSize=100&includeRemoved=true&spaces=drive",
            DRIVE_API_BASE,
            page_token
        );

        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| format!("Failed to get changes: {}", e))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Drive API error: {}", error_text));
        }

        response
            .json()
            .await
            .map_err(|e| format!("Failed to parse changes response: {}", e))
    }

    /// Check for new video files since last token
    /// Returns (new_video_files, new_token)
    pub async fn get_video_changes(&self, page_token: &str) -> Result<(Vec<DriveItem>, String), String> {
        let mut all_video_files = Vec::new();
        let mut current_token = page_token.to_string();

        let video_mimes = [
            "video/mp4",
            "video/x-matroska",
            "video/avi",
            "video/quicktime",
            "video/webm",
            "video/x-m4v",
            "video/x-ms-wmv",
            "video/x-flv",
            "video/mp2t",
        ];

        loop {
            let changes = self.get_changes(&current_token).await?;

            // Filter for video files that weren't removed
            for change in changes.changes {
                if change.removed.unwrap_or(false) {
                    continue;
                }

                if let Some(file) = change.file {
                    if video_mimes.contains(&file.mime_type.as_str()) {
                        all_video_files.push(file);
                    }
                }
            }

            // Check if we need to paginate
            if let Some(next_token) = changes.next_page_token {
                current_token = next_token;
            } else if let Some(new_token) = changes.new_start_page_token {
                // No more pages, return the new token for next time
                return Ok((all_video_files, new_token));
            } else {
                // Shouldn't happen, but use current token as fallback
                return Ok((all_video_files, current_token));
            }
        }
    }
}

// ==================== OAuth Flow ====================

/// Generate the OAuth authorization URL (via backend proxy)
pub fn get_auth_url() -> String {
    format!("{}/auth/google", AUTH_SERVER_URL)
}

/// Parse tokens from deep link callback URL
/// The backend sends tokens via: streamvault://oauth/callback?tokens=BASE64_ENCODED_JSON
pub fn parse_tokens_from_callback(url: &str) -> Result<GoogleTokens, String> {
    // Parse the URL to get the tokens parameter
    let url_parts: Vec<&str> = url.split('?').collect();
    if url_parts.len() < 2 {
        return Err("No query string in callback URL".to_string());
    }

    let query = url_parts[1];
    let params: HashMap<&str, &str> = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            Some((parts.next()?, parts.next()?))
        })
        .collect();

    // Check for error
    if let Some(error) = params.get("error") {
        return Err(format!("OAuth error: {}", error));
    }

    // Get and decode the tokens
    let tokens_b64 = params.get("tokens")
        .ok_or("No tokens in callback URL")?;

    let tokens_json = String::from_utf8(
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, tokens_b64)
            .map_err(|e| format!("Failed to decode tokens: {}", e))?
    ).map_err(|e| format!("Invalid UTF-8 in tokens: {}", e))?;

    let token_data: serde_json::Value = serde_json::from_str(&tokens_json)
        .map_err(|e| format!("Failed to parse tokens JSON: {}", e))?;

    let access_token = token_data["access_token"]
        .as_str()
        .ok_or("Missing access_token")?
        .to_string();

    let refresh_token = token_data["refresh_token"]
        .as_str()
        .map(String::from);

    let expires_in = token_data["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    let token_type = token_data["token_type"]
        .as_str()
        .unwrap_or("Bearer")
        .to_string();

    Ok(GoogleTokens {
        access_token,
        refresh_token,
        expires_at: Some(expires_at),
        token_type,
    })
}

/// Start local OAuth callback server and wait for tokens from backend
/// The backend exchanges the code and redirects here with tokens directly
pub async fn wait_for_oauth_callback() -> Result<GoogleTokens, String> {
    // Start a local TCP listener on the redirect URI port
    let listener = TcpListener::bind("127.0.0.1:8085")
        .map_err(|e| format!("Failed to start OAuth callback server: {}", e))?;

    println!("[GDRIVE] OAuth callback server listening on port 8085");

    // Set a timeout for the connection
    listener.set_nonblocking(false).ok();

    // Accept one connection
    let (mut stream, _) = listener
        .accept()
        .map_err(|e| format!("Failed to accept OAuth callback: {}", e))?;

    // Read the HTTP request
    let buf_reader = BufReader::new(&stream);
    let request_line = buf_reader
        .lines()
        .next()
        .ok_or("No request received")?
        .map_err(|e| format!("Failed to read request: {}", e))?;

    println!("[GDRIVE] Received callback: {}", request_line);

    // Parse the request to get tokens or error
    let tokens = extract_tokens_from_request(&request_line)?;

    // Send a success response
    let response_body = r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>Authorization Successful</title>
            <style>
                body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
                       display: flex; justify-content: center; align-items: center; height: 100vh;
                       margin: 0; background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); }
                .container { text-align: center; background: white; padding: 40px 60px;
                            border-radius: 16px; box-shadow: 0 10px 40px rgba(0,0,0,0.2); }
                h1 { color: #22c55e; margin-bottom: 10px; }
                p { color: #666; }
            </style>
        </head>
        <body>
            <div class="container">
                <h1>✓ Authorization Successful!</h1>
                <p>You can close this window and return to StreamVault.</p>
            </div>
        </body>
        </html>
    "#;

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );

    stream.write_all(response.as_bytes()).ok();
    stream.flush().ok();

    Ok(tokens)
}

/// Extract tokens from callback request
fn extract_tokens_from_request(request_line: &str) -> Result<GoogleTokens, String> {
    // Parse: GET /callback?tokens=BASE64_DATA HTTP/1.1
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err("Invalid request line".to_string());
    }

    let path = parts[1];

    // Check for error
    if path.contains("error=") {
        let query_start = path.find('?').ok_or("No query string")?;
        let query = &path[query_start + 1..];
        let params: HashMap<&str, &str> = query
            .split('&')
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                Some((parts.next()?, parts.next()?))
            })
            .collect();

        let error = params.get("error").unwrap_or(&"unknown_error");
        return Err(format!("OAuth error: {}", error));
    }

    // Parse query parameters
    let query_start = path.find('?').ok_or("No query string in callback URL")?;
    let query = &path[query_start + 1..];

    let params: HashMap<&str, &str> = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            Some((parts.next()?, parts.next()?))
        })
        .collect();

    // Get and decode the tokens
    let tokens_b64 = params.get("tokens")
        .ok_or("No tokens in callback URL")?;

    let tokens_json = String::from_utf8(
        base64::engine::general_purpose::STANDARD.decode(tokens_b64)
            .map_err(|e| format!("Failed to decode tokens: {}", e))?
    ).map_err(|e| format!("Invalid UTF-8 in tokens: {}", e))?;

    let token_data: serde_json::Value = serde_json::from_str(&tokens_json)
        .map_err(|e| format!("Failed to parse tokens JSON: {}", e))?;

    let access_token = token_data["access_token"]
        .as_str()
        .ok_or("Missing access_token")?
        .to_string();

    let refresh_token = token_data["refresh_token"]
        .as_str()
        .map(String::from);

    let expires_in = token_data["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    let token_type = token_data["token_type"]
        .as_str()
        .unwrap_or("Bearer")
        .to_string();

    Ok(GoogleTokens {
        access_token,
        refresh_token,
        expires_at: Some(expires_at),
        token_type,
    })
}

/// Exchange authorization code for tokens
/// NOTE: This function is deprecated. The new flow uses backend proxy
/// and tokens are returned via deep link callback.
/// Use parse_tokens_from_callback() instead.
#[deprecated(note = "Use parse_tokens_from_callback() with the new backend proxy flow")]
pub async fn exchange_code_for_tokens(_code: &str) -> Result<GoogleTokens, String> {
    Err("This function is deprecated. Tokens are now received via deep link callback from the backend proxy. Use parse_tokens_from_callback() to parse tokens from the callback URL.".to_string())
}

// ==================== Helpers ====================

fn get_tokens_path() -> PathBuf {
    get_app_data_dir().join("gdrive_tokens.json")
}

fn save_tokens(tokens: &GoogleTokens) -> Result<(), String> {
    let path = get_tokens_path();
    let json = serde_json::to_string_pretty(tokens)
        .map_err(|e| format!("Failed to serialize tokens: {}", e))?;

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }

    fs::write(&path, json).map_err(|e| format!("Failed to save tokens: {}", e))
}

fn load_tokens() -> Result<GoogleTokens, String> {
    let path = get_tokens_path();
    let json = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read tokens: {}", e))?;

    serde_json::from_str(&json).map_err(|e| format!("Failed to parse tokens: {}", e))
}

fn extract_auth_code(request_line: &str) -> Result<String, String> {
    // Parse: GET /callback?code=XXX&state=YYY HTTP/1.1
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err("Invalid request line".to_string());
    }

    let path = parts[1];

    // Check for error
    if path.contains("error=") {
        return Err("User denied authorization".to_string());
    }

    // Parse query parameters
    let query_start = path.find('?').ok_or("No query string in callback URL")?;
    let query = &path[query_start + 1..];

    let params: HashMap<&str, &str> = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            Some((parts.next()?, parts.next()?))
        })
        .collect();

    params
        .get("code")
        .map(|s| s.to_string())
        .ok_or("No code in callback URL".to_string())
}

fn generate_state() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
        .collect()
}

// ==================== URL Encoding Helper ====================

mod urlencoding {
    pub fn encode(input: &str) -> String {
        percent_encoding::utf8_percent_encode(
            input,
            percent_encoding::NON_ALPHANUMERIC
        ).to_string()
    }
}
