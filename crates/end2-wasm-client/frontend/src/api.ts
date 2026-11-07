const API = import.meta.env.VITE_API_URL ?? "/api";

export type ApiResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: ApiError };

export type ApiError = {
  status: number;
  message: string;
  detail?: string;
};

export const Ok = <T>(value: T): ApiResult<T> => ({
  ok: true,
  value,
});

export const Err = (error: ApiError): ApiResult<never> => ({
  ok: false,
  error,
});

export async function request<T>(
  endpoint: string,
  method: string,
  body: any = null,
): Promise<ApiResult<T>> {
  const options: RequestInit = {
    method,
    credentials: "include",
  };

  if (body) {
    options.body = JSON.stringify(body);
    options.headers = { "content-type": "application/json" };
  }

  const response = await fetch(`${API}${endpoint}`, options);
  if (!response.ok) {
    let error: ApiError = await response.json();
    return Err(error);
  }

  const text = await response.text();
  const json = text ? JSON.parse(text) : {};
  return Ok(json);
}
