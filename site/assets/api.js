// ClassNode API client — token storage, auto-refresh, thin fetch wrapper.
const API = {
  base: '/api/v1',

  get access() { return localStorage.getItem('cn_access'); },
  set access(v) { v ? localStorage.setItem('cn_access', v) : localStorage.removeItem('cn_access'); },
  get refresh() { return localStorage.getItem('cn_refresh'); },
  set refresh(v) { v ? localStorage.setItem('cn_refresh', v) : localStorage.removeItem('cn_refresh'); },

  async req(method, path, body, isForm) {
    const headers = {};
    if (this.access) headers.authorization = `Bearer ${this.access}`;
    if (body && !isForm) headers['content-type'] = 'application/json';
    const res = await fetch(this.base + path, {
      method, headers,
      body: body ? (isForm ? body : JSON.stringify(body)) : undefined,
    });
    if (res.status === 401 && this.refresh && !path.startsWith('/auth')) {
      if (await this.tryRefresh()) return this.req(method, path, body, isForm);
    }
    if (res.status === 204) return null;
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw Object.assign(new Error(data.error || res.statusText), { status: res.status });
    return data;
  },

  get: (p) => API.req('GET', p),
  post: (p, b) => API.req('POST', p, b),
  patch: (p, b) => API.req('PATCH', p, b),
  del: (p) => API.req('DELETE', p),
  form: (p, fd) => API.req('POST', p, fd, true),

  async login(email, password) {
    const d = await this.req('POST', '/auth/login', { email, password });
    this.access = d.access_token; this.refresh = d.refresh_token;
    return d;
  },
  async register(payload) {
    const d = await this.req('POST', '/auth/register', payload);
    this.access = d.access_token; this.refresh = d.refresh_token;
    return d;
  },
  async tryRefresh() {
    try {
      const d = await this.req('POST', '/auth/refresh', { refresh_token: this.refresh });
      this.access = d.access_token; this.refresh = d.refresh_token;
      return true;
    } catch { this.logout(); return false; }
  },
  logout() {
    if (this.refresh) this.req('POST', '/auth/logout', { refresh_token: this.refresh }).catch(() => {});
    this.access = null; this.refresh = null;
    location.hash = '#/login';
  },
  me: () => API.req('GET', '/auth/me'),
};
