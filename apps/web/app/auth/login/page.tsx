import Link from "next/link";
import { AuthMessage } from "@/components/AuthMessage";
import { login } from "@/app/auth/actions";

type Props = {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
};

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

export default async function LoginPage({ searchParams }: Props) {
  const params = await searchParams;

  return (
    <section className="form-wrap">
      <p className="eyebrow">Account</p>
      <h1 style={{ fontSize: "2.4rem" }}>Log in</h1>
      <AuthMessage error={first(params.error)} message={first(params.message)} />

      <form className="form" action={login}>
        <label>
          Email
          <input name="email" type="email" autoComplete="email" required />
        </label>
        <label>
          Password
          <input name="password" type="password" autoComplete="current-password" required />
        </label>
        <button type="submit">Log in</button>
      </form>

      <p style={{ marginTop: "1rem", color: "#666", fontSize: "0.9rem" }}>
        <Link href="/auth/forgot-password">Forgot password?</Link>
        {" · "}
        <Link href="/auth/signup">Create account</Link>
      </p>
    </section>
  );
}
