import type { ProviderLoginRequest } from '$api/types/auth_provider';
import type { CodeChallengeMethod } from '$api/types/authorize';

const AUTHORIZE_PATH = '/auth/v1/oidc/authorize';
const LOGIN_HINT = 'login_hint';

export function ownAuthorizeUrl(uri: string | undefined): URL | undefined {
    if (!uri) {
        return undefined;
    }
    try {
        const url = new URL(uri, window.location.origin);
        if (url.origin === window.location.origin && url.pathname === AUTHORIZE_PATH) {
            return url;
        }
    } catch {
        return undefined;
    }
    return undefined;
}

export function registerHref(authorizeUrl: string, email: string): string {
    const params = new URLSearchParams({ redirect_uri: authorizeUrl });
    if (email) {
        params.set(LOGIN_HINT, email);
    }
    return `/auth/v1/users/register?${params}`;
}

export function signInHref(authorize: URL, email: string): string {
    const url = new URL(authorize);
    if (email) {
        url.searchParams.set(LOGIN_HINT, email);
    } else {
        url.searchParams.delete(LOGIN_HINT);
    }
    return url.toString();
}

export function providerLoginForAuthorize(
    authorize: URL,
    providerId: string,
): ProviderLoginRequest | undefined {
    const params = authorize.searchParams;
    const clientId = params.get('client_id');
    const redirectUri = params.get('redirect_uri');
    if (!clientId || !redirectUri) {
        return undefined;
    }
    const method = params.get('code_challenge_method');
    return {
        email: undefined,
        client_id: clientId,
        redirect_uri: redirectUri,
        scopes: params.get('scope')?.split(' ') || [],
        state: params.get('state') || undefined,
        nonce: params.get('nonce') || undefined,
        code_challenge: params.get('code_challenge') || undefined,
        code_challenge_method:
            method === 'S256' || method === 'plain' ? (method as CodeChallengeMethod) : undefined,
        provider_id: providerId,
        pkce_challenge: '',
        pow: '',
    };
}
