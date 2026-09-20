"use server";

import { redirect } from "next/navigation";
import { appUrl } from "@/lib/env";
import { createClient } from "@/lib/supabase/server";

function textValue(formData: FormData, name: string): string {
  const value = formData.get(name);
  return typeof value === "string" ? value.trim() : "";
}

function withMessage(path: string, key: "error" | "message", value: string): never {
  redirect(`${path}?${key}=${encodeURIComponent(value)}`);
}

export async function login(formData: FormData) {
  const email = textValue(formData, "email");
  const password = textValue(formData, "password");
  const supabase = await createClient();

  const { error } = await supabase.auth.signInWithPassword({ email, password });
  if (error) withMessage("/auth/login", "error", error.message);

  redirect("/dashboard");
}

export async function signup(formData: FormData) {
  const email = textValue(formData, "email");
  const password = textValue(formData, "password");
  const accepted = formData.get("legal") === "yes";

  if (!accepted) {
    withMessage("/auth/signup", "error", "Please accept the Terms and Privacy Policy.");
  }

  if (password.length < 10) {
    withMessage("/auth/signup", "error", "Use at least 10 characters for the password.");
  }

  const supabase = await createClient();
  const { data, error } = await supabase.auth.signUp({
    email,
    password,
    options: {
      emailRedirectTo: `${appUrl()}/auth/callback?next=/dashboard`
    }
  });

  if (error) withMessage("/auth/signup", "error", error.message);
  if (data.session) redirect("/dashboard");

  withMessage("/auth/login", "message", "Check your email to confirm the account, then log in.");
}

export async function requestPasswordReset(formData: FormData) {
  const email = textValue(formData, "email");
  const supabase = await createClient();

  const { error } = await supabase.auth.resetPasswordForEmail(email, {
    redirectTo: `${appUrl()}/auth/callback?next=/auth/reset-password`
  });

  if (error) withMessage("/auth/forgot-password", "error", error.message);
  withMessage("/auth/login", "message", "If the account exists, a reset link has been sent.");
}

export async function resetPassword(formData: FormData) {
  const password = textValue(formData, "password");
  if (password.length < 10) {
    withMessage("/auth/reset-password", "error", "Use at least 10 characters for the password.");
  }

  const supabase = await createClient();
  const { error } = await supabase.auth.updateUser({ password });

  if (error) withMessage("/auth/reset-password", "error", error.message);
  withMessage("/auth/login", "message", "Password updated. Log in with the new password.");
}

export async function logout() {
  const supabase = await createClient();
  await supabase.auth.signOut();
  redirect("/");
}
