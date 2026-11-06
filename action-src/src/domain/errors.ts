export class InputError extends Error {
  readonly input: string;
  constructor(input: string, message: string) {
    super(message);
    this.name = 'InputError';
    this.input = input;
  }
}
