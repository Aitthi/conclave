import { User } from "./user";

export function authenticate(u: User): boolean {
  return u.name.length > 0;
}

export function revoke(u: User): boolean {
  return authenticate(u);
}
