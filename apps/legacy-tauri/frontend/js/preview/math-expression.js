/**
 * 受限数学表达式解析器，只接受数字、括号和显式运算符。
 */

const MAX_EXPRESSION_LENGTH = 100;
const NUMBER_PREFIX_RE = /^(?:\d+(?:\.\d*)?|\.\d+)/;

class ExpressionParser {
  constructor(expression) {
    this.expression = expression;
    this.position = 0;
  }

  evaluate() {
    const value = this.parseAdditive();
    this.skipWhitespace();
    if (this.position !== this.expression.length) {
      throw new SyntaxError("Unexpected token");
    }
    if (!Number.isFinite(value)) {
      throw new RangeError("Expression result is not finite");
    }
    return value;
  }

  parseAdditive() {
    let value = this.parseMultiplicative();
    while (true) {
      if (this.consume("+")) {
        value += this.parseMultiplicative();
      } else if (this.consume("-")) {
        value -= this.parseMultiplicative();
      } else {
        return value;
      }
    }
  }

  parseMultiplicative() {
    let value = this.parseUnary();
    while (true) {
      this.skipWhitespace();
      if (this.expression.startsWith("**", this.position)) {
        return value;
      }
      if (this.consume("*")) {
        value *= this.parseUnary();
      } else if (this.consume("/")) {
        value /= this.parseUnary();
      } else if (this.consume("%")) {
        value %= this.parseUnary();
      } else {
        return value;
      }
    }
  }

  parseUnary() {
    if (this.consume("+")) return this.parseUnary();
    if (this.consume("-")) return -this.parseUnary();
    return this.parsePower();
  }

  parsePower() {
    const value = this.parsePrimary();
    if (this.consume("^") || this.consume("**")) {
      return value ** this.parseUnary();
    }
    return value;
  }

  parsePrimary() {
    if (this.consume("(")) {
      const value = this.parseAdditive();
      if (!this.consume(")")) {
        throw new SyntaxError("Missing closing parenthesis");
      }
      return value;
    }
    return this.parseNumber();
  }

  parseNumber() {
    this.skipWhitespace();
    const remaining = this.expression.slice(this.position);
    const match = remaining.match(NUMBER_PREFIX_RE);
    if (!match) {
      throw new SyntaxError("Expected number");
    }
    this.position += match[0].length;
    return Number(match[0]);
  }

  consume(token) {
    this.skipWhitespace();
    if (!this.expression.startsWith(token, this.position)) return false;
    this.position += token.length;
    return true;
  }

  skipWhitespace() {
    while (/\s/.test(this.expression[this.position] || "")) {
      this.position += 1;
    }
  }
}

export function evaluateMathExpression(expression) {
  if (typeof expression !== "string" || expression.length > MAX_EXPRESSION_LENGTH) {
    throw new SyntaxError("Invalid expression");
  }
  return new ExpressionParser(expression).evaluate();
}
