export interface User {
  id: number;
  name: string;
}

export function newUser(id: number, name: string): User {
  return { id, name };
}

export function unusedHelper(): number {
  return 42;
}
