{ Bruto Pascal runtime: extended grapheme clusters, the user-perceived
  characters Free Vision draws one per cell (or two, for wide ones).

  The clustering follows the main rules of Unicode's UAX #29: a cluster
  is a base character followed by combining marks, variation selectors,
  emoji modifiers and zero-width-joiner sequences; CR LF and a pair of
  regional indicators (a flag) are one cluster each. Hangul syllable
  sequences and the Indic conjunct rules are not handled. FPC has the
  enumerator in its System unit, as a class; Bruto has no classes, so it
  is an object, which Bruto's for-in loops drive the same way. }
unit GraphemeBreakProperty;

interface

type
  TUnicodeStringExtendedGraphemeClustersEnumerator = object
    FStr: UnicodeString;
    FPos: Int64;
    Current: UnicodeString;
    constructor Create(const S: UnicodeString);
    function MoveNext: Boolean;
  end;

{ The code point at S[I] (1-based), and in Units how many UTF-16 units
  it takes. }
function CodePointAt(const S: UnicodeString; I: Int64; var Units: Int64): Int64;
{ Whether code point C continues the cluster before it. }
function IsGraphemeExtend(C: Int64): Boolean;
{ Whether code point C is shown two cells wide (East Asian Wide and
  Fullwidth characters, and most emoji). }
function IsWideCodePoint(C: Int64): Boolean;

implementation

function CodePointAt(const S: UnicodeString; I: Int64; var Units: Int64): Int64;
var
  Hi, Lo: Int64;
begin
  Hi := Ord(S[I]);
  Units := 1;
  CodePointAt := Hi;
  if (Hi >= $D800) and (Hi < $DC00) and (I < Length(S)) then
  begin
    Lo := Ord(S[I + 1]);
    if (Lo >= $DC00) and (Lo < $E000) then
    begin
      CodePointAt := $10000 + ((Hi - $D800) shl 10) + (Lo - $DC00);
      Units := 2;
    end;
  end;
end;

const
  { Ranges (first, last) of the code points that extend a cluster:
    combining marks, joiners, variation selectors, emoji modifiers and
    tags. }
  ExtendRanges: array[0..31] of LongInt = (
    $300, $36F, $483, $489, $591, $5BD, $610, $61A, $64B, $65F, $900, $903,
    $93A, $94F, $1AB0, $1AFF, $1DC0, $1DFF, $200C, $200D, $20D0, $20FF,
    $FE00, $FE0F, $FE20, $FE2F, $1F3FB, $1F3FF, $E0020, $E007F,
    $E0100, $E01EF);
  { Ranges of East Asian Wide and Fullwidth characters and of the
    emoji shown two cells wide. }
  WideRanges: array[0..115] of LongInt = (
    $1100, $115F, $231A, $231B, $2329, $232A, $23E9, $23EC, $23F0, $23F0,
    $23F3, $23F3, $25FD, $25FE, $2614, $2615, $2648, $2653, $267F, $267F,
    $2693, $2693, $26A1, $26A1, $26AA, $26AB, $26BD, $26BE, $26C4, $26C5,
    $26CE, $26CE, $26D4, $26D4, $26EA, $26EA, $26F2, $26F3, $26F5, $26F5,
    $26FA, $26FA, $26FD, $26FD, $2705, $2705, $270A, $270B, $2728, $2728,
    $274C, $274C, $274E, $274E, $2753, $2755, $2757, $2757, $2795, $2797,
    $27B0, $27B0, $27BF, $27BF, $2B1B, $2B1C, $2B50, $2B50, $2B55, $2B55,
    $2E80, $303E, $3041, $33FF, $3400, $4DBF, $4E00, $9FFF, $A000, $A4CF,
    $A960, $A97F, $AC00, $D7A3, $F900, $FAFF, $FE10, $FE19, $FE30, $FE6F,
    $FF00, $FF60, $FFE0, $FFE6, $1F004, $1F004, $1F0CF, $1F0CF,
    $1F18E, $1F18E, $1F191, $1F19A, $1F1E6, $1F1FF, $1F200, $1F2FF,
    $1F300, $1F64F, $1F680, $1F6FF, $1F900, $1F9FF, $1FA70, $1FAFF,
    $20000, $3FFFD);

function IsGraphemeExtend(C: Int64): Boolean;
var
  I: Integer;
begin
  IsGraphemeExtend := False;
  I := 0;
  while I < High(ExtendRanges) do
  begin
    if (C >= ExtendRanges[I]) and (C <= ExtendRanges[I + 1]) then
    begin
      IsGraphemeExtend := True;
      Exit;
    end;
    I := I + 2;
  end;
end;

function IsRegionalIndicator(C: Int64): Boolean;
begin
  IsRegionalIndicator := (C >= $1F1E6) and (C <= $1F1FF);
end;

function IsWideCodePoint(C: Int64): Boolean;
var
  I: Integer;
begin
  IsWideCodePoint := False;
  I := 0;
  while I < High(WideRanges) do
  begin
    if (C >= WideRanges[I]) and (C <= WideRanges[I + 1]) then
    begin
      IsWideCodePoint := True;
      Exit;
    end;
    I := I + 2;
  end;
end;

constructor TUnicodeStringExtendedGraphemeClustersEnumerator.Create(const S: UnicodeString);
begin
  FStr := S;
  FPos := 1;
  Current := '';
end;

function TUnicodeStringExtendedGraphemeClustersEnumerator.MoveNext: Boolean;
var
  Start, Units, C, Prev: Int64;
  Done: Boolean;
begin
  if FPos > Length(FStr) then
  begin
    Current := '';
    MoveNext := False;
    Exit;
  end;
  Start := FPos;
  C := CodePointAt(FStr, FPos, Units);
  FPos := FPos + Units;
  if (C = 13) and (FPos <= Length(FStr)) and (Ord(FStr[FPos]) = 10) then
    FPos := FPos + 1
  else if (C <> 10) and (C <> 13) then
  begin
    Prev := C;
    Done := False;
    while (not Done) and (FPos <= Length(FStr)) do
    begin
      C := CodePointAt(FStr, FPos, Units);
      if IsGraphemeExtend(C) or (Prev = $200D) or
        (IsRegionalIndicator(Prev) and IsRegionalIndicator(C) and (FPos - Start = 2)) then
      begin
        FPos := FPos + Units;
        Prev := C;
      end
      else
        Done := True;
    end;
  end;
  Current := Copy(FStr, Start, FPos - Start);
  MoveNext := True;
end;

end.
