export function AuthMessage({
  error,
  message
}: {
  error?: string;
  message?: string;
}) {
  if (error) return <div className="notice error" role="alert">{error}</div>;
  if (message) return <div className="notice success" role="status">{message}</div>;
  return null;
}
