const ESC = "\u001b";

/**
 * Repère une séquence d'échappement ANSI incomplète en fin de texte, pour ne
 * pas la laisser fuiter quand un flux est découpé en morceaux arbitraires.
 */
function incompleteEscapeIndex(text: string): number {
  const start = text.lastIndexOf(ESC);
  if (start === -1) {
    return -1;
  }
  const rest = text.slice(start + 1);
  if (rest.length === 0) {
    return start;
  }
  const first = rest[0];
  if (first === "[") {
    for (let index = 1; index < rest.length; index += 1) {
      const code = rest.charCodeAt(index);
      if (code >= 0x40 && code <= 0x7e) {
        return -1;
      }
    }
    return start;
  }
  if (first === "]") {
    if (rest.indexOf("\u0007") === -1 && rest.indexOf(`${ESC}\\`) === -1) {
      return start;
    }
    return -1;
  }
  if (first === "P" || first === "X" || first === "^" || first === "_") {
    if (rest.indexOf(`${ESC}\\`) === -1) {
      return start;
    }
    return -1;
  }
  if (first === "(" || first === ")" || first === "#" || first === "%") {
    return rest.length < 2 ? start : -1;
  }
  return -1;
}

function applyBackspaces(text: string): string {
  let output = "";
  for (const char of text) {
    if (char === "\b") {
      if (output.length > 0 && !output.endsWith("\n")) {
        output = output.slice(0, -1);
      }
    } else {
      output += char;
    }
  }
  return output;
}

/**
 * Retire les séquences ANSI/contrôle d'un texte destiné à un `<pre>`.
 * Aucun HTML n'est produit : le résultat est toujours du texte brut.
 */
export function stripAnsi(input: string): string {
  let text = input;
  // OSC (titres de fenêtre, hyperliens) terminés par BEL ou ST.
  text = text.replace(/\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)?/g, "");
  // DCS / SOS / PM / APC.
  text = text.replace(/\u001b[P^_X][^\u001b]*(?:\u001b\\)?/g, "");
  // CSI (couleurs, déplacements de curseur).
  text = text.replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, "");
  // Échappements à deux caractères et sélecteurs de jeu.
  text = text.replace(/\u001b[()#%][0-9A-Za-z]?/g, "");
  text = text.replace(/\u001b[@-_]/g, "");
  // Retours chariot : CRLF devient LF, un CR isolé (barre de progression) est retiré.
  text = text.replace(/\r\n/g, "\n").replace(/\r/g, "");
  text = applyBackspaces(text);
  // Contrôles C0 restants (sauf tabulation et saut de ligne) et DEL.
  text = text.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "");
  return text;
}

/**
 * Filtre ANSI incrémental : conserve les séquences incomplètes d'un morceau à
 * l'autre pour qu'elles ne s'affichent jamais sous forme de caractères bruts.
 */
export class AnsiFilter {
  private pending = "";

  push(chunk: string): string {
    let text = this.pending + chunk;
    this.pending = "";
    const cut = incompleteEscapeIndex(text);
    if (cut >= 0) {
      this.pending = text.slice(cut);
      text = text.slice(0, cut);
    }
    return stripAnsi(text);
  }

  /** Abandonne une séquence incomplète en fin de flux. */
  flush(): string {
    this.pending = "";
    return "";
  }

  reset(): void {
    this.pending = "";
  }
}
