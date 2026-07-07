const express = require('express');
const crypto = require('crypto');
require('dotenv').config();

const app = express();
app.disable('x-powered-by');
app.use(express.json());

const PORT = process.env.PORT || 3001;
const OAUTH_CALLBACK_BASE = process.env.OAUTH_CALLBACK_URL || 'http://localhost:8085/callback';

// Google OAuth credentials (stored securely on server)
const GOOGLE_CLIENT_ID = process.env.GOOGLE_CLIENT_ID;
const GOOGLE_CLIENT_SECRET = process.env.GOOGLE_CLIENT_SECRET;

// Auto-detect redirect URI from request
const getRedirectUri = (req) => {
  const protocol = req.headers['x-forwarded-proto'] || req.protocol || 'https';
  const host = req.headers['x-forwarded-host'] || req.headers.host;

  if (host && (host.startsWith('localhost') || host.startsWith('127.'))) {
    return `${protocol}://${host}/auth/callback`;
  }
  if (process.env.OAUTH_REDIRECT_URL) return process.env.OAUTH_REDIRECT_URL;
  if (process.env.REDIRECT_URI) return process.env.REDIRECT_URI;
  return `${protocol}://${host}/auth/callback`;
};

const DRIVE_SCOPES = [
  'https://www.googleapis.com/auth/drive',
  'https://www.googleapis.com/auth/userinfo.email'
].join(' ');

// ========== In-memory OAuth stores ==========

const oauthSessionStore = new Map();
const oauthStateStore = new Map();
const OAUTH_SESSION_TTL = 300000; // 5 minutes

// Cleanup expired OAuth sessions
setInterval(() => {
  const now = Date.now();
  for (const [id, entry] of oauthSessionStore) {
    if (now - entry.createdAt > OAUTH_SESSION_TTL) oauthSessionStore.delete(id);
  }
  for (const [state, entry] of oauthStateStore) {
    if (now - entry.createdAt > OAUTH_SESSION_TTL) oauthStateStore.delete(state);
  }
}, 30000);

// ========== API ==========

app.get('/', (req, res) => {
  res.json({ service: 'SlasshyVault Auth Server', version: '2.0.0' });
});

app.get('/health', (req, res) => {
  res.json({
    status: 'ok',
    configured: !!(GOOGLE_CLIENT_ID && GOOGLE_CLIENT_SECRET),
  });
});

// Token session retrieval (called by Tauri app after OAuth callback)
app.get('/auth/session/:sessionId', (req, res) => {
  const { sessionId } = req.params;
  const session = oauthSessionStore.get(sessionId);
  if (!session) return res.status(404).json({ error: 'Session not found or expired' });
  oauthSessionStore.delete(sessionId);

  res.json({
    access_token: session.access_token,
    refresh_token: session.refresh_token,
    expires_in: session.expires_in,
    token_type: session.token_type,
    nonce: session.nonce,
  });
});

// ========== OAuth helpers ==========

function redirectToGoogleAuth(req, res, scopes) {
  const redirectUri = getRedirectUri(req);

  if (!GOOGLE_CLIENT_ID) {
    return res.status(500).json({ error: 'GOOGLE_CLIENT_ID not configured' });
  }

  const nonce = req.query.nonce;
  if (!nonce || typeof nonce !== 'string' || nonce.length < 16 || nonce.length > 128) {
    return res.status(400).json({ error: 'A valid nonce parameter is required (16-128 chars)' });
  }

  const state = crypto.randomUUID();
  oauthStateStore.set(state, { createdAt: Date.now(), nonce });

  const authUrl = new URL('https://accounts.google.com/o/oauth2/v2/auth');
  authUrl.searchParams.set('client_id', GOOGLE_CLIENT_ID);
  authUrl.searchParams.set('redirect_uri', redirectUri);
  authUrl.searchParams.set('response_type', 'code');
  authUrl.searchParams.set('scope', scopes);
  authUrl.searchParams.set('access_type', 'offline');
  authUrl.searchParams.set('prompt', 'consent');
  authUrl.searchParams.set('state', state);

  return res.redirect(authUrl.toString());
}

// Step 1: Initiate OAuth flow
app.get('/auth/google', (req, res) => {
  redirectToGoogleAuth(req, res, DRIVE_SCOPES);
});

// Step 2: Handle Google callback
app.get('/auth/callback', async (req, res) => {
  const redirectUri = getRedirectUri(req);
  const { code, error, state } = req.query;

  if (error) {
    return res.redirect(`${OAUTH_CALLBACK_BASE}?error=${encodeURIComponent(error)}`);
  }

  if (!state) return res.redirect(`${OAUTH_CALLBACK_BASE}?error=invalid_state`);

  const stateData = oauthStateStore.get(state);
  if (!stateData) return res.redirect(`${OAUTH_CALLBACK_BASE}?error=invalid_state`);
  oauthStateStore.delete(state);

  if (!code) return res.redirect(`${OAUTH_CALLBACK_BASE}?error=no_code`);

  try {
    const tokenResponse = await fetch('https://oauth2.googleapis.com/token', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        client_id: GOOGLE_CLIENT_ID,
        client_secret: GOOGLE_CLIENT_SECRET,
        code: code,
        grant_type: 'authorization_code',
        redirect_uri: redirectUri,
      }),
    });

    const tokens = await tokenResponse.json();

    if (tokens.error) {
      console.error('Token error from Google OAuth:', tokens.error);
      return res.redirect(`${OAUTH_CALLBACK_BASE}?error=${encodeURIComponent(tokens.error_description || tokens.error)}`);
    }

    const sessionId = crypto.randomUUID();
    oauthSessionStore.set(sessionId, {
      access_token: tokens.access_token,
      refresh_token: tokens.refresh_token,
      expires_in: tokens.expires_in,
      token_type: tokens.token_type,
      nonce: stateData.nonce || '',
      createdAt: Date.now(),
    });

    const nonceParam = stateData.nonce ? `&nonce=${encodeURIComponent(stateData.nonce)}` : '';
    res.redirect(`${OAUTH_CALLBACK_BASE}?session_id=${sessionId}${nonceParam}`);

  } catch (err) {
    console.error('Token exchange error:', err);
    res.redirect(`${OAUTH_CALLBACK_BASE}?error=token_exchange_failed`);
  }
});

// Step 3: Refresh token
app.post('/auth/refresh', async (req, res) => {
  const { refresh_token } = req.body;
  if (!refresh_token) return res.status(400).json({ error: 'refresh_token required' });

  try {
    const tokenResponse = await fetch('https://oauth2.googleapis.com/token', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        client_id: GOOGLE_CLIENT_ID,
        client_secret: GOOGLE_CLIENT_SECRET,
        refresh_token: refresh_token,
        grant_type: 'refresh_token',
      }),
    });

    const tokens = await tokenResponse.json();
    if (tokens.error) return res.status(400).json({ error: tokens.error });

    res.json({ access_token: tokens.access_token, expires_in: tokens.expires_in, token_type: tokens.token_type, refresh_token: tokens.refresh_token });
  } catch (err) {
    console.error('Refresh error:', err);
    res.status(500).json({ error: 'refresh_failed' });
  }
});

// ========== Start ==========

app.listen(PORT, () => {
  console.log(`[Auth] SlasshyVault Auth Server running on port ${PORT}`);
  console.log(`[Auth] OAuth configured: ${!!(GOOGLE_CLIENT_ID && GOOGLE_CLIENT_SECRET)}`);
});
