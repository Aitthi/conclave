import { authenticate } from "./auth";
import { newUser, User } from "./user";

export function login(name: string): User | null {
  const u = newUser(1, name);
  return authenticate(u) ? u : null;
}

export function whoami(u: User): string {
  return u.name;
}
