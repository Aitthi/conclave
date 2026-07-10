export interface User {
  id: string;
  name: string;
}

export type Status = "active" | "inactive";

export class UserRepo {
  list(): User[] {
    return [];
  }
}

export function findUser(id: string): User | undefined {
  return undefined;
}
