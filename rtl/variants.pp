{ Bruto Pascal runtime: FPC's Variants unit.

  A Variant is FPC's TVarData (declared in System): a type code and 8
  bytes of value. Strings (varString, and varOleStr holding a
  UnicodeString) and variant arrays are counted like strings, through
  bruto_var_addref and bruto_var_release. A variant array is a dynamic
  array of variants (see "Variant arrays" below); it is copied when an
  element of a shared one is assigned, so assigning an array variant
  copies it, as FPC does.

  The compiler calls the bruto_var_* routines for conversions, operators
  and indexing, with pointers to TVarData; a routine filling Res leaves
  it holding its own reference. The conversion and operator rules are
  FPC's (cvarutil.inc and variants.pp). }
unit Variants;

{$mode objfpc}{$H+}

interface

uses
  SysUtils, TypInfo;

type
  EVariantParamNotFoundError = class(EVariantError);
  EVariantInvalidOpError = class(EVariantError);
  EVariantTypeCastError = class(EVariantError);
  EVariantOverflowError = class(EVariantError);
  EVariantInvalidArgError = class(EVariantError);
  EVariantBadVarTypeError = class(EVariantError);
  EVariantBadIndexError = class(EVariantError);
  EVariantArrayLockedError = class(EVariantError);
  EVariantNotAnArrayError = class(EVariantError);
  EVariantArrayCreateError = class(EVariantError);
  EVariantNotImplError = class(EVariantError);
  EVariantOutOfMemoryError = class(EVariantError);
  EVariantUnexpectedError = class(EVariantError);
  EVariantDispatchError = class(EVariantError);
  EVariantRangeCheckError = class(EVariantOverflowError);
  EVariantInvalidNullOpError = class(EVariantInvalidOpError);

  TVariantRelationship = (vrEqual, vrLessThan, vrGreaterThan, vrNotEqual);
  TNullCompareRule = (ncrError, ncrStrict, ncrLoose);

  TVarOp = (opAdd, opSubtract, opMultiply, opDivide, opIntDivide, opModulus,
    opShiftLeft, opShiftRight, opAnd, opOr, opXor, opCompare, opNegate,
    opNot, opCmpEq, opCmpNe, opCmpLt, opCmpLe, opCmpGt, opCmpGe, opPower);

var
  NullEqualityRule: TNullCompareRule = ncrLoose;
  NullMagnitudeRule: TNullCompareRule = ncrLoose;
  NullStrictConvert: Boolean = True;
  NullAsStringValue: string = '';

function VarType(const V: Variant): TVarType;
function VarTypeAsText(const AType: TVarType): string;
function VarAsType(const V: Variant; AVarType: TVarType): Variant;
function VarIsType(const V: Variant; AVarType: TVarType): Boolean;
function VarIsByRef(const V: Variant): Boolean;
function VarIsEmpty(const V: Variant): Boolean;
function VarIsNull(const V: Variant): Boolean;
function VarIsClear(const V: Variant): Boolean;
function VarIsOrdinal(const V: Variant): Boolean;
function VarIsFloat(const V: Variant): Boolean;
function VarIsNumeric(const V: Variant): Boolean;
function VarIsStr(const V: Variant): Boolean;
function VarIsBool(const V: Variant): Boolean;
function VarIsArray(const A: Variant): Boolean;
function VarToStr(const V: Variant): string;
function VarToStrDef(const V: Variant; const ADefault: string): string;
function VarToWideStr(const V: Variant): UnicodeString;
function VarToUnicodeStr(const V: Variant): UnicodeString;
function VarSameValue(const A, B: Variant): Boolean;
function VarCompareValue(const A, B: Variant): TVariantRelationship;
function VarInRange(const AValue, AMin, AMax: Variant): Boolean;
procedure VarClear(var V: Variant);
procedure VarCopy(var Dest: Variant; const Source: Variant);
procedure VarCast(var Dest: Variant; const Source: Variant; AVarType: LongInt);
function VarArrayCreate(const Bounds: array of SizeInt; AVarType: TVarType): Variant;
function VarArrayOf(const Values: array of Variant): Variant;
function VarArrayDimCount(const A: Variant): LongInt;
function VarArrayLowBound(const A: Variant; Dim: LongInt): LongInt;
function VarArrayHighBound(const A: Variant; Dim: LongInt): LongInt;
procedure VarArrayRedim(var A: Variant; HighBound: SizeInt);
function Unassigned: Variant;
function Null: Variant;

procedure VarCastError;
procedure VarCastError(const ASourceType, ADestType: TVarType);
procedure VarInvalidOp;
procedure VarInvalidOp(const ALeft, ARight: TVarType; AOpCode: TVarOp);
procedure VarInvalidArgError;
procedure VarBadIndexError;

function GetVariantProp(Instance: TObject; PropInfo: PPropInfo): Variant;
function GetVariantProp(Instance: TObject; const PropName: string): Variant;
procedure SetVariantProp(Instance: TObject; PropInfo: PPropInfo; const Value: Variant);
procedure SetVariantProp(Instance: TObject; const PropName: string; const Value: Variant);
function GetPropValue(Instance: TObject; const PropName: string): Variant;
function GetPropValue(Instance: TObject; const PropName: string; PreferStrings: Boolean): Variant;
procedure SetPropValue(Instance: TObject; const PropName: string; const Value: Variant);

{ Compiler entry points. }
procedure bruto_var_from_int(Res: Pointer; V: Int64; VType: Int64);
procedure bruto_var_from_double(Res: Pointer; V: Double);
procedure bruto_var_from_curr(Res: Pointer; C: Int64);
procedure bruto_var_from_str(Res: Pointer; S: Pointer);
procedure bruto_var_from_ustr(Res: Pointer; S: Pointer);
procedure bruto_var_from_bool(Res: Pointer; B: Int64);
function bruto_var_to_int(P: Pointer): Int64;
function bruto_var_to_double(P: Pointer): Double;
function bruto_var_to_curr(P: Pointer): Int64;
function bruto_var_to_bool(P: Pointer): Int64;
function bruto_var_to_str(P: Pointer): AnsiString;
function bruto_var_to_ustr(P: Pointer): UnicodeString;
procedure bruto_var_op(Res, L, R: Pointer; Op: Int64);
function bruto_var_cmp(L, R: Pointer; Op: Int64): Int64;
procedure bruto_var_unary(Res, P: Pointer; Op: Int64);
procedure bruto_var_array_get(Res, A, Indices: Pointer; Count: Int64);
procedure bruto_var_array_put(A, Indices: Pointer; Count: Int64; V: Pointer);

implementation

const
  VAR_TYPEMISMATCH = LongInt($80020005);
  VAR_BADINDEX = LongInt($8002000B);
  VAR_INVALIDARG = LongInt($80070057);

  { FPC's common types, to which operands are mapped to pick an
    operation. }
  ctEmpty = 0;
  ctAny = 1;
  ctError = 2;
  ctLongInt = 3;
  ctBoolean = 4;
  ctFloat = 5;
  ctDate = 6;
  ctCurrency = 7;
  ctInt64 = 8;
  ctNull = 9;
  ctWideStr = 10;
  ctString = 11;

  VarTypeToCommonType: array[0..21] of Byte = (
    ctEmpty, ctNull, ctLongInt, ctLongInt, ctFloat, ctFloat, ctCurrency,
    ctDate, ctWideStr, ctError, ctError, ctBoolean, ctError, ctError,
    ctError, ctError, ctLongInt, ctLongInt, ctLongInt, ctInt64, ctInt64,
    ctInt64);

  FindCmpCommonType: array[0..11, 0..11] of Byte = (
    (ctEmpty, ctEmpty, ctError, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty),
    (ctEmpty, ctAny, ctError, ctAny, ctAny, ctAny, ctAny, ctAny, ctAny, ctAny, ctAny, ctAny),
    (ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError),
    (ctEmpty, ctAny, ctError, ctLongInt, ctBoolean, ctFloat, ctDate, ctCurrency, ctInt64, ctNull, ctFloat, ctFloat),
    (ctEmpty, ctAny, ctError, ctLongInt, ctBoolean, ctFloat, ctDate, ctCurrency, ctInt64, ctNull, ctWideStr, ctString),
    (ctEmpty, ctAny, ctError, ctFloat, ctFloat, ctFloat, ctDate, ctCurrency, ctFloat, ctNull, ctFloat, ctFloat),
    (ctEmpty, ctAny, ctError, ctDate, ctDate, ctDate, ctDate, ctDate, ctDate, ctNull, ctDate, ctDate),
    (ctEmpty, ctAny, ctError, ctCurrency, ctCurrency, ctCurrency, ctDate, ctCurrency, ctCurrency, ctNull, ctCurrency, ctCurrency),
    (ctEmpty, ctAny, ctError, ctInt64, ctInt64, ctFloat, ctDate, ctCurrency, ctInt64, ctNull, ctFloat, ctFloat),
    (ctEmpty, ctAny, ctError, ctNull, ctNull, ctNull, ctNull, ctNull, ctNull, ctNull, ctNull, ctNull),
    (ctEmpty, ctAny, ctError, ctFloat, ctWideStr, ctFloat, ctDate, ctCurrency, ctFloat, ctNull, ctWideStr, ctWideStr),
    (ctEmpty, ctAny, ctError, ctFloat, ctString, ctFloat, ctDate, ctCurrency, ctFloat, ctNull, ctWideStr, ctString));

  FindOpCommonType: array[0..11, 0..11] of Byte = (
    (ctEmpty, ctAny, ctError, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty, ctEmpty),
    (ctAny, ctAny, ctError, ctAny, ctAny, ctAny, ctAny, ctAny, ctAny, ctAny, ctAny, ctAny),
    (ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError, ctError),
    (ctEmpty, ctAny, ctError, ctLongInt, ctBoolean, ctFloat, ctDate, ctCurrency, ctInt64, ctNull, ctFloat, ctFloat),
    (ctEmpty, ctAny, ctError, ctLongInt, ctBoolean, ctFloat, ctDate, ctCurrency, ctInt64, ctNull, ctBoolean, ctBoolean),
    (ctEmpty, ctAny, ctError, ctFloat, ctFloat, ctFloat, ctDate, ctCurrency, ctFloat, ctNull, ctFloat, ctFloat),
    (ctEmpty, ctAny, ctError, ctDate, ctDate, ctDate, ctDate, ctDate, ctDate, ctNull, ctDate, ctDate),
    (ctEmpty, ctAny, ctError, ctCurrency, ctCurrency, ctCurrency, ctDate, ctCurrency, ctCurrency, ctNull, ctCurrency, ctCurrency),
    (ctEmpty, ctAny, ctError, ctInt64, ctInt64, ctFloat, ctDate, ctCurrency, ctInt64, ctNull, ctFloat, ctFloat),
    (ctEmpty, ctAny, ctError, ctNull, ctNull, ctNull, ctNull, ctNull, ctNull, ctNull, ctNull, ctNull),
    (ctEmpty, ctAny, ctError, ctFloat, ctBoolean, ctFloat, ctDate, ctCurrency, ctFloat, ctNull, ctWideStr, ctWideStr),
    (ctEmpty, ctAny, ctError, ctFloat, ctBoolean, ctFloat, ctDate, ctCurrency, ctFloat, ctNull, ctWideStr, ctString));

  VarOpAsText: array[TVarOp] of string = ('+', '-', '*', '/', 'div', 'mod',
    'shl', 'shr', 'and', 'or', 'xor', '', '-', 'not', '=', '<>', '<', '<=',
    '>', '>=', '**');

{ ── Errors ─────────────────────────────────────────────────── }

procedure VariantTypeMismatch;
begin
  raise EVariantError.CreateCode(VAR_TYPEMISMATCH);
end;

procedure VarCastError;
begin
  raise EVariantTypeCastError.Create('Invalid variant type cast');
end;

procedure VarCastError(const ASourceType, ADestType: TVarType);
begin
  raise EVariantTypeCastError.Create('Could not convert variant of type ('
    + VarTypeAsText(ASourceType) + ') into type (' + VarTypeAsText(ADestType) + ')');
end;

procedure VarInvalidOp;
begin
  raise EVariantInvalidOpError.Create('Invalid variant operation');
end;

procedure VarInvalidOp(const ALeft, ARight: TVarType; AOpCode: TVarOp);
begin
  raise EVariantInvalidOpError.Create('Invalid variant operation '
    + VarTypeAsText(ALeft) + ' ' + VarOpAsText[AOpCode] + ' ' + VarTypeAsText(ARight));
end;

procedure VarInvalidUnaryOp(const AType: TVarType; AOpCode: TVarOp);
begin
  raise EVariantInvalidOpError.Create('Invalid variant operation '
    + VarOpAsText[AOpCode] + ' ' + VarTypeAsText(AType));
end;

procedure VarInvalidNullOp;
begin
  raise EVariantInvalidNullOpError.Create('Invalid NULL variant operation');
end;

procedure VarInvalidArgError;
begin
  raise EVariantInvalidArgError.CreateCode(VAR_INVALIDARG);
end;

procedure VarBadIndexError;
begin
  raise EVariantBadIndexError.CreateCode(VAR_BADINDEX);
end;

{ ── Storage ────────────────────────────────────────────────── }

{ Point D (holding nothing counted) at the counted P. }
procedure SetCounted(var D: TVarData; AType: TVarType; P: Pointer);
begin
  bruto_astr_addref(P);
  D.VType := AType;
  D.VString := P;
end;

{ Empty D, releasing what it held. As FPC's, only a string's or array's
  pointer is cleared: a number's bytes stay (so a cleared variant
  property is still streamed, as nil). }
procedure ClearData(var D: TVarData);
begin
  if (D.VType = varString) or (D.VType = varOleStr) or (D.VType = varUString)
    or ((D.VType and varArray) <> 0) then
  begin
    bruto_var_release(@D);
    D.VString := nil;
  end;
  D.VType := varEmpty;
end;

{ Make D a copy of S (D holding nothing counted). }
procedure CopyData(var D: TVarData; const S: TVarData);
begin
  Move(S, D, SizeOf(TVarData));
  bruto_var_addref(@D);
end;

procedure SetStr(var D: TVarData; const S: AnsiString);
begin
  ClearData(D);
  if S = '' then
  begin
    D.VType := varString;
    D.VString := nil;
  end
  else
    SetCounted(D, varString, Pointer(S));
end;

procedure SetUStr(var D: TVarData; const S: UnicodeString);
begin
  ClearData(D);
  if S = '' then
  begin
    D.VType := varOleStr;
    D.VOleStr := nil;
  end
  else
    SetCounted(D, varOleStr, Pointer(S));
end;

procedure SetInt64(var D: TVarData; V: Int64);
begin
  ClearData(D);
  D.VType := varInt64;
  D.VInt64 := V;
end;

procedure SetInteger(var D: TVarData; V: LongInt);
begin
  ClearData(D);
  D.VType := varInteger;
  D.VInt64 := 0;
  D.VInteger := V;
end;

procedure SetDouble(var D: TVarData; V: Double);
begin
  ClearData(D);
  D.VType := varDouble;
  D.VDouble := V;
end;

procedure SetBool(var D: TVarData; V: Boolean);
begin
  ClearData(D);
  D.VType := varBoolean;
  D.VInt64 := 0;
  D.VBoolean := V;
end;

{ ── Variant arrays ─────────────────────────────────────────── }

{ A variant array's data is a dynamic array of TVarData: first a header
  entry holding the number of dimensions, then one per dimension (its
  low bound in Reserved2..3, its length in the value), then the
  elements, the last index varying fastest, as in FPC's (resizing the
  last dimension keeps the elements in place, so they move to other
  indices, as in FPC). The header entries are empty variants, so
  releasing and counting the block's entries passes over them. }

type
  TVarDataArray = array[0..$FFFFFF] of TVarData;
  PVarDataArray = ^TVarDataArray;
  TInt64Array = array[0..$FFFF] of Int64;
  PInt64Array = ^TInt64Array;

procedure ReleaseElements(P: Pointer; Count: Int64);
var
  I: Int64;
begin
  I := 0;
  while I < Count do
  begin
    bruto_var_release(@PVarDataArray(P)^[I]);
    I := I + 1;
  end;
end;

procedure AddrefElements(P: Pointer; Count: Int64);
var
  I: Int64;
begin
  I := 0;
  while I < Count do
  begin
    bruto_var_addref(@PVarDataArray(P)^[I]);
    I := I + 1;
  end;
end;

function DimsOf(Data: Pointer): Int64;
begin
  Result := PVarDataArray(Data)^[0].VInt64;
end;

function DimLow(Data: Pointer; Dim: Int64): Int64;
begin
  Result := PLongInt(@PVarDataArray(Data)^[Dim].Reserved2)^;
end;

function DimCount(Data: Pointer; Dim: Int64): Int64;
begin
  Result := PVarDataArray(Data)^[Dim].VInt64;
end;

{ The number of elements of array data Data. }
function ElementCount(Data: Pointer): Int64;
var
  I: Int64;
begin
  Result := 1;
  for I := 1 to DimsOf(Data) do
    Result := Result * DimCount(Data, I);
end;

{ New array data with dimensions of low bounds Lows and lengths Counts,
  each element of type ElemType (empty when that is varVariant),
  holding its own reference. }
function NewArrayData(const Lows, Counts: array of Int64; ElemType: TVarType): Pointer;
var
  Dims, Total, I: Int64;
begin
  Dims := Length(Lows);
  Total := 1;
  for I := 0 to Dims - 1 do
    Total := Total * Counts[I];
  Result := bruto_dynarr_owned(1 + Dims + Total, SizeOf(TVarData), @ReleaseElements,
    @AddrefElements);
  PVarDataArray(Result)^[0].VInt64 := Dims;
  for I := 0 to Dims - 1 do
  begin
    PLongInt(@PVarDataArray(Result)^[I + 1].Reserved2)^ := Lows[I];
    PVarDataArray(Result)^[I + 1].VInt64 := Counts[I];
  end;
  if ElemType <> varVariant then
    for I := 1 + Dims to Dims + Total do
      PVarDataArray(Result)^[I].VType := ElemType;
end;

{ A copy of array data Data (of elements of type ElemType), its last
  dimension LastCount long, holding its own reference. }
function CopyArrayData(Data: Pointer; LastCount: Int64; ElemType: TVarType): Pointer;
var
  Dims, I, OldTotal, NewTotal: Int64;
  Lows, Counts: array of Int64;
begin
  Dims := DimsOf(Data);
  SetLength(Lows, Dims);
  SetLength(Counts, Dims);
  for I := 1 to Dims do
  begin
    Lows[I - 1] := DimLow(Data, I);
    Counts[I - 1] := DimCount(Data, I);
  end;
  OldTotal := ElementCount(Data);
  Counts[Dims - 1] := LastCount;
  Result := NewArrayData(Lows, Counts, ElemType);
  NewTotal := ElementCount(Result);
  I := 0;
  while (I < OldTotal) and (I < NewTotal) do
  begin
    Move(PVarDataArray(Data)^[1 + Dims + I], PVarDataArray(Result)^[1 + Dims + I],
      SizeOf(TVarData));
    bruto_var_addref(@PVarDataArray(Result)^[1 + Dims + I]);
    I := I + 1;
  end;
end;

{ The element of array D at the first of the Count Indices (as many as
  it has dimensions), checked. }
function ElementAt(const D: TVarData; Indices: PInt64Array; Count: Int64): PVarData;
var
  Data: Pointer;
  Dims, I, Flat, Stride, K: Int64;
begin
  if (D.VType and varArray) = 0 then
    VarInvalidArgError;
  Data := D.VArray;
  Dims := DimsOf(Data);
  if Count < Dims then
    VarInvalidArgError;
  Flat := 0;
  Stride := 1;
  for I := Dims downto 1 do
  begin
    K := Indices^[I - 1] - DimLow(Data, I);
    if (K < 0) or (K >= DimCount(Data, I)) then
      VarBadIndexError;
    Flat := Flat + K * Stride;
    Stride := Stride * DimCount(Data, I);
  end;
  Result := @PVarDataArray(Data)^[1 + Dims + Flat];
end;

{ Make the array in D its own (copied when it is shared). }
procedure UniqueArray(var D: TVarData);
var
  Old: Pointer;
begin
  if PLongInt(D.VArray - 12)^ > 1 then
  begin
    Old := D.VArray;
    D.VArray := CopyArrayData(Old, DimCount(Old, DimsOf(Old)), D.VType and varTypeMask);
    bruto_astr_release(Old);
  end;
end;

{ ── Conversions ────────────────────────────────────────────── }

function StrToBoolValue(const S: string; out B: Boolean): Boolean;
var
  I: Int64;
  D: Double;
  Code: Integer;
begin
  Result := True;
  if CompareText(S, 'True') = 0 then
    B := True
  else if CompareText(S, 'False') = 0 then
    B := False
  else
  begin
    Val(S, I, Code);
    if Code = 0 then
      B := I <> 0
    else
    begin
      Val(S, D, Code);
      if Code = 0 then
        B := D <> 0
      else
        Result := False;
    end;
  end;
end;

function StrToInt64Value(const S: string): Int64;
var
  Code: Integer;
begin
  Val(S, Result, Code);
  if Code <> 0 then
    VariantTypeMismatch;
end;

function StrToDoubleValue(const S: string): Double;
var
  Code: Integer;
begin
  Val(S, Result, Code);
  if Code <> 0 then
    VariantTypeMismatch;
end;

function VariantToInt64(const D: TVarData): Int64;
begin
  case D.VType of
    varEmpty: Result := 0;
    varSmallInt: Result := D.VSmallInt;
    varShortInt: Result := D.VShortInt;
    varInteger: Result := D.VInteger;
    varSingle: Result := Round(D.VSingle);
    varDouble, varDate: Result := Round(D.VDouble);
    varCurrency: Result := Round(D.VCurrency);
    varBoolean: if D.VBoolean then Result := -1 else Result := 0;
    varByte: Result := D.VByte;
    varWord: Result := D.VWord;
    varLongWord: Result := D.VLongWord;
    varInt64: Result := D.VInt64;
    varQWord: Result := Int64(D.VQWord);
    varOleStr, varUString: Result := StrToInt64Value(UnicodeString(D.VOleStr));
    varString: Result := StrToInt64Value(AnsiString(D.VString));
  else
    VariantTypeMismatch;
  end;
end;

function VariantToDouble(const D: TVarData): Double;
begin
  case D.VType of
    varEmpty: Result := 0;
    varSmallInt: Result := D.VSmallInt;
    varShortInt: Result := D.VShortInt;
    varInteger: Result := D.VInteger;
    varSingle: Result := D.VSingle;
    varDouble, varDate: Result := D.VDouble;
    varCurrency: Result := D.VCurrency;
    varBoolean: if D.VBoolean then Result := -1 else Result := 0;
    varByte: Result := D.VByte;
    varWord: Result := D.VWord;
    varLongWord: Result := D.VLongWord;
    varInt64: Result := D.VInt64;
    varQWord: Result := D.VQWord;
    varOleStr, varUString: Result := StrToDoubleValue(UnicodeString(D.VOleStr));
    varString: Result := StrToDoubleValue(AnsiString(D.VString));
  else
    VariantTypeMismatch;
  end;
end;

function VariantToCurrency(const D: TVarData): Currency;
begin
  case D.VType of
    varCurrency: PInt64(@Result)^ := D.VInt64;
    varSingle, varDouble, varDate, varOleStr, varUString, varString:
      Result := VariantToDouble(D);
  else
    Result := VariantToInt64(D);
  end;
end;

function VariantToBoolean(const D: TVarData): Boolean;
begin
  case D.VType of
    varEmpty: Result := False;
    varSmallInt: Result := D.VSmallInt <> 0;
    varShortInt: Result := D.VShortInt <> 0;
    varInteger: Result := D.VInteger <> 0;
    varSingle: Result := D.VSingle <> 0;
    varDouble, varDate: Result := D.VDouble <> 0;
    varCurrency: Result := D.VInt64 <> 0;
    varBoolean: Result := D.VBoolean;
    varByte: Result := D.VByte <> 0;
    varWord: Result := D.VWord <> 0;
    varLongWord: Result := D.VLongWord <> 0;
    varInt64: Result := D.VInt64 <> 0;
    varQWord: Result := D.VQWord <> 0;
    varOleStr, varUString:
      if not StrToBoolValue(UnicodeString(D.VOleStr), Result) then
        VariantTypeMismatch;
    varString:
      if not StrToBoolValue(AnsiString(D.VString), Result) then
        VariantTypeMismatch;
  else
    VariantTypeMismatch;
  end;
end;

function VariantToAnsiString(const D: TVarData): AnsiString;
begin
  case D.VType of
    varEmpty: Result := '';
    varSmallInt: Result := IntToStr(D.VSmallInt);
    varShortInt: Result := IntToStr(D.VShortInt);
    varInteger: Result := IntToStr(D.VInteger);
    varSingle: Result := FloatToStr(D.VSingle);
    varDouble, varDate: Result := FloatToStr(D.VDouble);
    varCurrency: Result := CurrToStr(D.VCurrency);
    varBoolean: Result := BoolToStr(D.VBoolean, True);
    varByte: Result := IntToStr(D.VByte);
    varWord: Result := IntToStr(D.VWord);
    varLongWord: Result := IntToStr(D.VLongWord);
    varInt64: Result := IntToStr(D.VInt64);
    varQWord: Result := IntToStr(D.VQWord);
    varOleStr, varUString: Result := AnsiString(UnicodeString(D.VOleStr));
    varString: Result := AnsiString(D.VString);
  else
    VariantTypeMismatch;
  end;
end;

function VariantToUnicodeString(const D: TVarData): UnicodeString;
begin
  if (D.VType = varOleStr) or (D.VType = varUString) then
    Result := UnicodeString(D.VOleStr)
  else
    Result := UnicodeString(VariantToAnsiString(D));
end;

{ D converted to type AType, into R (holding nothing counted). }
procedure CastData(var R: TVarData; const D: TVarData; AType: TVarType);
var
  I: Int64;
begin
  if D.VType = AType then
  begin
    CopyData(R, D);
    Exit;
  end;
  if D.VType = varNull then
  begin
    if AType = varEmpty then
      Exit;
    VarCastError(varNull, AType);
  end;
  if (D.VType and varArray) <> 0 then
    VarCastError(D.VType, AType);
  case AType of
    varEmpty: R.VType := varEmpty;
    varNull: R.VType := varNull;
    varSmallInt, varShortInt, varInteger, varByte, varWord, varLongWord,
    varInt64, varQWord:
      begin
        I := VariantToInt64(D);
        R.VType := AType;
        R.VInt64 := 0;
        case AType of
          varSmallInt: R.VSmallInt := I;
          varShortInt: R.VShortInt := I;
          varInteger: R.VInteger := I;
          varByte: R.VByte := I;
          varWord: R.VWord := I;
          varLongWord: R.VLongWord := I;
        else
          R.VInt64 := I;
        end;
      end;
    varSingle:
      begin
        R.VType := varSingle;
        R.VInt64 := 0;
        R.VSingle := VariantToDouble(D);
      end;
    varDouble, varDate:
      begin
        R.VType := AType;
        R.VDouble := VariantToDouble(D);
      end;
    varCurrency:
      begin
        R.VType := varCurrency;
        R.VCurrency := VariantToCurrency(D);
      end;
    varBoolean: SetBool(R, VariantToBoolean(D));
    varString: SetStr(R, VariantToAnsiString(D));
    varOleStr, varUString: SetUStr(R, VariantToUnicodeString(D));
  else
    VarCastError(D.VType, AType);
  end;
end;

{ ── Public routines ────────────────────────────────────────── }

function VarType(const V: Variant): TVarType;
begin
  Result := PVarData(@V)^.VType;
end;

function VarTypeAsText(const AType: TVarType): string;
const
  Names: array[0..21] of string = ('Empty', 'Null', 'Smallint', 'Integer',
    'Single', 'Double', 'Currency', 'Date', 'OleStr', 'Dispatch', 'Error',
    'Boolean', 'Variant', 'Unknown', 'Decimal', '???', 'ShortInt', 'Byte',
    'Word', 'DWord', 'Int64', 'QWord');
var
  T: TVarType;
begin
  T := AType and varTypeMask;
  if T <= 21 then
    Result := Names[T]
  else if T = varString then
    Result := 'String'
  else if T = varAny then
    Result := 'Any'
  else if T = varUString then
    Result := 'UnicodeString'
  else
    Result := '$' + IntToHex(T, 4);
  if (AType and varArray) <> 0 then
    Result := 'Array of ' + Result;
  if (AType and varByRef) <> 0 then
    Result := 'Ref to ' + Result;
end;

function VarAsType(const V: Variant; AVarType: TVarType): Variant;
begin
  CastData(PVarData(@Result)^, PVarData(@V)^, AVarType);
end;

procedure VarCast(var Dest: Variant; const Source: Variant; AVarType: LongInt);
var
  T: TVarData;
begin
  FillChar(T, SizeOf(T), 0);
  CastData(T, PVarData(@Source)^, AVarType);
  ClearData(PVarData(@Dest)^);
  Move(T, PVarData(@Dest)^, SizeOf(TVarData));
end;

function VarIsType(const V: Variant; AVarType: TVarType): Boolean;
begin
  Result := VarType(V) = AVarType;
end;

function VarIsByRef(const V: Variant): Boolean;
begin
  Result := (VarType(V) and varByRef) <> 0;
end;

function VarIsEmpty(const V: Variant): Boolean;
begin
  Result := VarType(V) = varEmpty;
end;

function VarIsNull(const V: Variant): Boolean;
begin
  Result := VarType(V) = varNull;
end;

function VarIsClear(const V: Variant): Boolean;
begin
  Result := VarType(V) = varEmpty;
end;

function VarIsOrdinal(const V: Variant): Boolean;
begin
  case VarType(V) of
    varSmallInt, varInteger, varBoolean, varShortInt, varByte, varWord,
    varLongWord, varInt64, varQWord: Result := True;
  else
    Result := False;
  end;
end;

function VarIsFloat(const V: Variant): Boolean;
begin
  case VarType(V) of
    varSingle, varDouble, varCurrency: Result := True;
  else
    Result := False;
  end;
end;

function VarIsNumeric(const V: Variant): Boolean;
begin
  Result := VarIsOrdinal(V) or VarIsFloat(V);
end;

function VarIsStr(const V: Variant): Boolean;
begin
  case VarType(V) of
    varOleStr, varString, varUString: Result := True;
  else
    Result := False;
  end;
end;

function VarIsBool(const V: Variant): Boolean;
begin
  Result := VarType(V) = varBoolean;
end;

function VarIsArray(const A: Variant): Boolean;
begin
  Result := (VarType(A) and varArray) <> 0;
end;

function VarToStr(const V: Variant): string;
begin
  Result := VarToStrDef(V, '');
end;

function VarToStrDef(const V: Variant; const ADefault: string): string;
begin
  if (VarType(V) = varNull) or (VarType(V) = varEmpty) then
    Result := ADefault
  else
    Result := VariantToAnsiString(PVarData(@V)^);
end;

function VarToWideStr(const V: Variant): UnicodeString;
begin
  if VarType(V) = varNull then
    Result := ''
  else
    Result := VariantToUnicodeString(PVarData(@V)^);
end;

function VarToUnicodeStr(const V: Variant): UnicodeString;
begin
  Result := VarToWideStr(V);
end;

procedure VarClear(var V: Variant);
begin
  ClearData(PVarData(@V)^);
end;

procedure VarCopy(var Dest: Variant; const Source: Variant);
begin
  Dest := Source;
end;

function Unassigned: Variant;
begin
  PVarData(@Result)^.VType := varEmpty;
end;

function Null: Variant;
begin
  PVarData(@Result)^.VType := varNull;
end;

function VarArrayCreate(const Bounds: array of SizeInt; AVarType: TVarType): Variant;
var
  D: PVarData;
  Lows, Counts: array of Int64;
  I, Dims: Integer;
begin
  if (Length(Bounds) = 0) or Odd(Length(Bounds)) then
    raise EVariantArrayCreateError.Create('Variant array cannot be created');
  Dims := Length(Bounds) div 2;
  SetLength(Lows, Dims);
  SetLength(Counts, Dims);
  for I := 0 to Dims - 1 do
  begin
    if Bounds[I * 2 + 1] < Bounds[I * 2] - 1 then
      raise EVariantArrayCreateError.Create('Variant array cannot be created');
    Lows[I] := Bounds[I * 2];
    Counts[I] := Bounds[I * 2 + 1] - Bounds[I * 2] + 1;
  end;
  D := PVarData(@Result);
  D^.VType := varArray or AVarType;
  D^.VArray := NewArrayData(Lows, Counts, AVarType);
end;

function VarArrayOf(const Values: array of Variant): Variant;
var
  I: Integer;
begin
  Result := VarArrayCreate([0, High(Values)], varVariant);
  for I := 0 to High(Values) do
    Result[I] := Values[I];
end;

function VarArrayDimCount(const A: Variant): LongInt;
begin
  if VarIsArray(A) then
    Result := DimsOf(PVarData(@A)^.VArray)
  else
    Result := 0;
end;

function VarArrayLowBound(const A: Variant; Dim: LongInt): LongInt;
begin
  if not VarIsArray(A) then
    VarInvalidArgError;
  if (Dim < 1) or (Dim > VarArrayDimCount(A)) then
    VarBadIndexError;
  Result := DimLow(PVarData(@A)^.VArray, Dim);
end;

function VarArrayHighBound(const A: Variant; Dim: LongInt): LongInt;
begin
  Result := VarArrayLowBound(A, Dim) + DimCount(PVarData(@A)^.VArray, Dim) - 1;
end;

procedure VarArrayRedim(var A: Variant; HighBound: SizeInt);
var
  D: PVarData;
  Old: Pointer;
  Dims: Int64;
begin
  if not VarIsArray(A) then
    VarInvalidArgError;
  D := PVarData(@A);
  Old := D^.VArray;
  Dims := DimsOf(Old);
  D^.VArray := CopyArrayData(Old, HighBound - DimLow(Old, Dims) + 1, D^.VType and varTypeMask);
  bruto_astr_release(Old);
end;

{ ── Operators ──────────────────────────────────────────────── }

function CommonType(const D: TVarData): Integer;
begin
  if D.VType <= 21 then
    Result := VarTypeToCommonType[D.VType]
  else if D.VType = varString then
    Result := ctString
  else if D.VType = varUString then
    Result := ctWideStr
  else if D.VType = varAny then
    Result := ctAny
  else
    Result := ctError;
end;

procedure OpFloat(var R: TVarData; const L, Rt: TVarData; Op: TVarOp);
var
  A, B: Double;
begin
  A := VariantToDouble(L);
  B := VariantToDouble(Rt);
  case Op of
    opAdd: A := A + B;
    opSubtract: A := A - B;
    opMultiply: A := A * B;
    opDivide: A := A / B;
  else
    VarInvalidOp(L.VType, Rt.VType, Op);
  end;
  SetDouble(R, A);
end;

procedure OpInt64(var R: TVarData; const L, Rt: TVarData; Op: TVarOp);
var
  A, B, C: Int64;
  Overflow: Boolean;
begin
  A := VariantToInt64(L);
  B := VariantToInt64(Rt);
  Overflow := False;
  C := 0;
  case Op of
    opAdd:
      begin
        C := A + B;
        Overflow := ((A xor C) and (B xor C)) < 0;
      end;
    opSubtract:
      begin
        C := A - B;
        Overflow := ((A xor B) and (A xor C)) < 0;
      end;
    opMultiply:
      begin
        C := A * B;
        Overflow := (A <> 0) and ((C div A <> B) or ((A = -1) and (B = Low(Int64))));
      end;
    opIntDivide: C := A div B;
    opModulus: C := A mod B;
    opShiftLeft: C := A shl B;
    opShiftRight: C := A shr B;
    opAnd: C := A and B;
    opOr: C := A or B;
    opXor: C := A xor B;
  else
    VarInvalidOp(L.VType, Rt.VType, Op);
  end;
  if Overflow then
    OpFloat(R, L, Rt, Op)
  else
    SetInt64(R, C);
end;

procedure OpInt64To32(var R: TVarData; const L, Rt: TVarData; Op: TVarOp);
begin
  OpInt64(R, L, Rt, Op);
  if (R.VType = varInt64) and (R.VInt64 >= Low(LongInt)) and (R.VInt64 <= High(LongInt)) then
    SetInteger(R, R.VInt64);
end;

procedure OpLongInt(var R: TVarData; const L, Rt: TVarData; Op: TVarOp);
var
  A, B: LongInt;
begin
  A := VariantToInt64(L);
  B := VariantToInt64(Rt);
  case Op of
    opIntDivide: A := A div B;
    opModulus: A := A mod B;
    opShiftLeft: A := A shl B;
    opShiftRight: A := LongInt(LongWord(A) shr B);
    opAnd: A := A and B;
    opOr: A := A or B;
    opXor: A := A xor B;
  else
    VarInvalidOp(L.VType, Rt.VType, Op);
  end;
  SetInteger(R, A);
end;

procedure OpBool(var R: TVarData; const L, Rt: TVarData; Op: TVarOp);
var
  A, B: Boolean;
begin
  A := VariantToBoolean(L);
  B := VariantToBoolean(Rt);
  case Op of
    opAnd: A := A and B;
    opOr: A := A or B;
    opXor: A := A xor B;
  else
    VarInvalidOp(L.VType, Rt.VType, Op);
  end;
  SetBool(R, A);
end;

procedure OpNull(var R: TVarData; const L, Rt: TVarData; Op: TVarOp);
begin
  if (Op = opAnd) or (Op = opOr) then
  begin
    if L.VType = varNull then
    begin
      R.VType := varNull;
      if Rt.VType <> varNull then
        if (Op = opAnd) <> VariantToBoolean(Rt) then
        begin
          ClearData(R);
          CopyData(R, Rt);
        end;
    end
    else
    begin
      CopyData(R, L);
      if (Op = opAnd) = VariantToBoolean(L) then
      begin
        ClearData(R);
        R.VType := varNull;
      end;
    end;
  end
  else
    R.VType := varNull;
end;

procedure OpCurr(var R: TVarData; const L, Rt: TVarData; Op: TVarOp; LCT, RCT: Integer);
var
  C: Currency;
  D: Double;
begin
  case Op of
    opAdd: C := VariantToCurrency(L) + VariantToCurrency(Rt);
    opSubtract: C := VariantToCurrency(L) - VariantToCurrency(Rt);
    opMultiply:
      if LCT = ctCurrency then
      begin
        if RCT = ctCurrency then
          C := VariantToCurrency(L) * VariantToCurrency(Rt)
        else
          C := VariantToCurrency(L) * VariantToDouble(Rt);
      end
      else if RCT = ctCurrency then
        C := VariantToDouble(L) * VariantToCurrency(Rt)
      else
        VarInvalidOp(L.VType, Rt.VType, Op);
    opDivide:
      if LCT = ctCurrency then
      begin
        if RCT = ctCurrency then
          C := VariantToCurrency(L) / VariantToCurrency(Rt)
        else
          C := VariantToCurrency(L) / VariantToDouble(Rt);
      end
      else if RCT = ctCurrency then
      begin
        D := VariantToCurrency(L) / VariantToCurrency(Rt);
        SetDouble(R, D);
        Exit;
      end
      else
        VarInvalidOp(L.VType, Rt.VType, Op);
  else
    VarInvalidOp(L.VType, Rt.VType, Op);
  end;
  ClearData(R);
  R.VType := varCurrency;
  R.VCurrency := C;
end;

procedure DoVarOp(var R: TVarData; const L, Rt: TVarData; Op: TVarOp);
var
  LCT, RCT: Integer;
begin
  LCT := CommonType(L);
  RCT := CommonType(Rt);
  case FindOpCommonType[LCT, RCT] of
    ctEmpty:
      case Op of
        opDivide: raise EZeroDivide.Create('Division by zero');
        opIntDivide, opModulus: raise EDivByZero.Create('Division by zero');
      end;
    ctLongInt:
      case Op of
        opAdd, opSubtract, opMultiply, opPower: OpInt64To32(R, L, Rt, Op);
        opDivide: OpFloat(R, L, Rt, Op);
      else
        OpLongInt(R, L, Rt, Op);
      end;
    ctFloat, ctDate:
      if Op in [opAdd, opSubtract, opMultiply, opDivide] then
        OpFloat(R, L, Rt, Op)
      else
        OpInt64To32(R, L, Rt, Op);
    ctCurrency:
      if Op in [opAdd, opSubtract, opMultiply, opDivide, opPower] then
        OpCurr(R, L, Rt, Op, LCT, RCT)
      else
        OpInt64To32(R, L, Rt, Op);
    ctBoolean:
      case Op of
        opAdd, opSubtract, opMultiply, opDivide, opPower: OpFloat(R, L, Rt, Op);
        opIntDivide, opModulus, opShiftLeft, opShiftRight: OpLongInt(R, L, Rt, Op);
        opAnd, opOr, opXor: OpBool(R, L, Rt, Op);
      else
        VarInvalidOp(L.VType, Rt.VType, Op);
      end;
    ctInt64:
      if Op <> opDivide then
        OpInt64(R, L, Rt, Op)
      else
        OpFloat(R, L, Rt, Op);
    ctNull: OpNull(R, L, Rt, Op);
    ctWideStr:
      case Op of
        opAdd: SetUStr(R, VariantToUnicodeString(L) + VariantToUnicodeString(Rt));
        opSubtract, opMultiply, opDivide, opPower: OpFloat(R, L, Rt, Op);
        opIntDivide..opXor: OpInt64To32(R, L, Rt, Op);
      else
        VarInvalidOp(L.VType, Rt.VType, Op);
      end;
    ctString:
      case Op of
        opAdd: SetStr(R, VariantToAnsiString(L) + VariantToAnsiString(Rt));
        opSubtract, opMultiply, opDivide, opPower: OpFloat(R, L, Rt, Op);
        opIntDivide..opXor: OpInt64To32(R, L, Rt, Op);
      else
        VarInvalidOp(L.VType, Rt.VType, Op);
      end;
  else
    VarInvalidOp(L.VType, Rt.VType, Op);
  end;
end;

{ A Currency's scaled Int64. }
function PInt64Of(C: Currency): Int64;
begin
  Result := PInt64(@C)^;
end;

function CmpSimple(L, R, Common: Integer): Integer;
begin
  if L = Common then
  begin
    if R = Common then
      Result := 0
    else
      Result := -1;
  end
  else
    Result := 1;
end;

function CmpInt64(A, B: Int64): Integer;
begin
  if A < B then
    Result := -1
  else if A > B then
    Result := 1
  else
    Result := 0;
end;

function CmpFloat(A, B: Double; Op: TVarOp): Integer;
begin
  if A = B then
    Result := 0
  else if (Op = opCmpEq) or (Op = opCmpNe) or (A < B) then
    Result := -1
  else
    Result := 1;
end;

function CmpNull(L, R: Integer; Op: TVarOp): Integer;
const
  ResultMap: array[Boolean, 0..5] of Integer = ((-1, 0, 0, 1, 0, -1), (0, -1, -1, -1, 1, 1));
begin
  if (Op = opCmpEq) or (Op = opCmpNe) then
    case NullEqualityRule of
      ncrError: VarInvalidNullOp;
      ncrStrict: Result := ResultMap[False, Ord(Op) - Ord(opCmpEq)];
    else
      Result := ResultMap[(L = R) xor (Op = opCmpNe), Ord(Op) - Ord(opCmpEq)];
    end
  else
    case NullMagnitudeRule of
      ncrError: VarInvalidNullOp;
      ncrStrict: Result := ResultMap[False, Ord(Op) - Ord(opCmpEq)];
    else
      Result := CmpSimple(L, R, ctNull);
    end;
end;

function CmpStr(const A, B: AnsiString; Op: TVarOp): Integer;
begin
  if ((Op = opCmpEq) or (Op = opCmpNe)) and (Length(A) <> Length(B)) then
    Result := -1
  else if A < B then
    Result := -1
  else if A > B then
    Result := 1
  else
    Result := 0;
end;

function CmpUStr(const A, B: UnicodeString; Op: TVarOp): Integer;
begin
  if ((Op = opCmpEq) or (Op = opCmpNe)) and (Length(A) <> Length(B)) then
    Result := -1
  else if A < B then
    Result := -1
  else if A > B then
    Result := 1
  else
    Result := 0;
end;

function DoVarCmp(const L, R: TVarData; Op: TVarOp): Integer;
var
  LCT, RCT: Integer;
begin
  LCT := CommonType(L);
  RCT := CommonType(R);
  case FindCmpCommonType[LCT, RCT] of
    ctEmpty: Result := CmpSimple(LCT, RCT, ctEmpty);
    ctLongInt, ctInt64: Result := CmpInt64(VariantToInt64(L), VariantToInt64(R));
    ctFloat, ctDate: Result := CmpFloat(VariantToDouble(L), VariantToDouble(R), Op);
    ctCurrency: Result := CmpInt64(PInt64Of(VariantToCurrency(L)), PInt64Of(VariantToCurrency(R)));
    ctBoolean: Result := CmpInt64(Ord(VariantToBoolean(L)), Ord(VariantToBoolean(R)));
    ctNull: Result := CmpNull(LCT, RCT, Op);
    ctWideStr: Result := CmpUStr(VariantToUnicodeString(L), VariantToUnicodeString(R), Op);
    ctString: Result := CmpStr(VariantToAnsiString(L), VariantToAnsiString(R), Op);
  else
    VarInvalidOp(L.VType, R.VType, Op);
  end;
end;

function CmpResult(C: Integer; Op: TVarOp): Boolean;
begin
  case Op of
    opCmpEq: Result := C = 0;
    opCmpNe: Result := C <> 0;
    opCmpLt: Result := C < 0;
    opCmpLe: Result := C <= 0;
    opCmpGt: Result := C > 0;
  else
    Result := C >= 0;
  end;
end;

function VarSameValue(const A, B: Variant): Boolean;
var
  DA, DB: PVarData;
begin
  DA := PVarData(@A);
  DB := PVarData(@B);
  if DA^.VType <> DB^.VType then
    Result := (DA^.VType = varEmpty) = (DB^.VType = varEmpty)
      and (DA^.VType <> varNull) and (DB^.VType <> varNull)
      and (DoVarCmp(DA^, DB^, opCmpEq) = 0)
  else if (DA^.VType = varEmpty) or (DA^.VType = varNull) then
    Result := True
  else
    Result := DoVarCmp(DA^, DB^, opCmpEq) = 0;
end;

function VarCompareValue(const A, B: Variant): TVariantRelationship;
var
  DA, DB: PVarData;
  C: Integer;
begin
  DA := PVarData(@A);
  DB := PVarData(@B);
  if (DA^.VType = varNull) or (DB^.VType = varNull) then
  begin
    if DA^.VType = DB^.VType then
      Result := vrEqual
    else
      Result := vrNotEqual;
    Exit;
  end;
  C := DoVarCmp(DA^, DB^, opCmpLt);
  if C = 0 then
    Result := vrEqual
  else if C < 0 then
    Result := vrLessThan
  else
    Result := vrGreaterThan;
end;

function VarInRange(const AValue, AMin, AMax: Variant): Boolean;
begin
  Result := (AValue >= AMin) and (AValue <= AMax);
end;

{ ── Compiler entry points ──────────────────────────────────── }

procedure bruto_var_from_int(Res: Pointer; V: Int64; VType: Int64);
var
  D: PVarData;
begin
  D := PVarData(Res);
  D^.VInt64 := 0;
  { A value too wide for its declared type: an Int64. }
  if (VType = varInteger) and ((V < Low(LongInt)) or (V > High(LongInt))) then
    VType := varInt64;
  D^.VType := VType;
  case VType of
    varShortInt: D^.VShortInt := V;
    varByte: D^.VByte := V;
    varSmallInt: D^.VSmallInt := V;
    varWord: D^.VWord := V;
    varInteger: D^.VInteger := V;
    varLongWord: D^.VLongWord := V;
  else
    D^.VInt64 := V;
  end;
end;

procedure bruto_var_from_double(Res: Pointer; V: Double);
begin
  PVarData(Res)^.VType := varDouble;
  PVarData(Res)^.VDouble := V;
end;

procedure bruto_var_from_curr(Res: Pointer; C: Int64);
begin
  PVarData(Res)^.VType := varCurrency;
  PVarData(Res)^.VInt64 := C;
end;

procedure bruto_var_from_str(Res: Pointer; S: Pointer);
begin
  SetCounted(PVarData(Res)^, varString, S);
end;

procedure bruto_var_from_ustr(Res: Pointer; S: Pointer);
begin
  SetCounted(PVarData(Res)^, varOleStr, S);
end;

procedure bruto_var_from_bool(Res: Pointer; B: Int64);
begin
  PVarData(Res)^.VType := varBoolean;
  PVarData(Res)^.VInt64 := 0;
  PVarData(Res)^.VBoolean := B <> 0;
end;

function bruto_var_to_int(P: Pointer): Int64;
begin
  if PVarData(P)^.VType = varNull then
  begin
    if NullStrictConvert then
      VarCastError(varNull, varInt64);
    Result := 0;
  end
  else
    Result := VariantToInt64(PVarData(P)^);
end;

function bruto_var_to_double(P: Pointer): Double;
begin
  if PVarData(P)^.VType = varNull then
  begin
    if NullStrictConvert then
      VarCastError(varNull, varDouble);
    Result := 0;
  end
  else
    Result := VariantToDouble(PVarData(P)^);
end;

function bruto_var_to_curr(P: Pointer): Int64;
begin
  if PVarData(P)^.VType = varNull then
  begin
    if NullStrictConvert then
      VarCastError(varNull, varCurrency);
    Result := 0;
  end
  else
    Result := PInt64Of(VariantToCurrency(PVarData(P)^));
end;

function bruto_var_to_bool(P: Pointer): Int64;
begin
  if PVarData(P)^.VType = varNull then
  begin
    if NullStrictConvert then
      VarCastError(varNull, varBoolean);
    Result := 0;
  end
  else
    Result := Ord(VariantToBoolean(PVarData(P)^));
end;

function bruto_var_to_str(P: Pointer): AnsiString;
begin
  if PVarData(P)^.VType = varNull then
  begin
    if NullStrictConvert then
      VarCastError(varNull, varString);
    Result := NullAsStringValue;
  end
  else
    Result := VariantToAnsiString(PVarData(P)^);
end;

function bruto_var_to_ustr(P: Pointer): UnicodeString;
begin
  if PVarData(P)^.VType = varNull then
  begin
    if NullStrictConvert then
      VarCastError(varNull, varOleStr);
    Result := UnicodeString(NullAsStringValue);
  end
  else
    Result := VariantToUnicodeString(PVarData(P)^);
end;

procedure bruto_var_op(Res, L, R: Pointer; Op: Int64);
begin
  DoVarOp(PVarData(Res)^, PVarData(L)^, PVarData(R)^, TVarOp(Op));
end;

function bruto_var_cmp(L, R: Pointer; Op: Int64): Int64;
begin
  Result := Ord(CmpResult(DoVarCmp(PVarData(L)^, PVarData(R)^, TVarOp(Op)), TVarOp(Op)));
end;

procedure bruto_var_unary(Res, P: Pointer; Op: Int64);
var
  D, R: PVarData;
  I: Int64;
begin
  D := PVarData(P);
  R := PVarData(Res);
  if TVarOp(Op) = opNegate then
    case D^.VType of
      varEmpty: bruto_var_from_int(Res, 0, varSmallInt);
      varNull: R^.VType := varNull;
      varSmallInt: bruto_var_from_int(Res, -D^.VSmallInt, varSmallInt);
      varInteger: bruto_var_from_int(Res, -Int64(D^.VInteger), varInteger);
      varShortInt: bruto_var_from_int(Res, -D^.VShortInt, varShortInt);
      varByte: bruto_var_from_int(Res, -D^.VByte, varSmallInt);
      varWord: bruto_var_from_int(Res, -D^.VWord, varInteger);
      varLongWord: bruto_var_from_int(Res, -Int64(D^.VLongWord), varInteger);
      varInt64: bruto_var_from_int(Res, -D^.VInt64, varInt64);
      varQWord: bruto_var_from_int(Res, -Int64(D^.VQWord), varInt64);
      varBoolean: bruto_var_from_int(Res, -Ord(D^.VBoolean), varSmallInt);
      varSingle:
        begin
          R^.VType := varSingle;
          R^.VSingle := -D^.VSingle;
        end;
      varDouble, varDate:
        begin
          R^.VType := D^.VType;
          R^.VDouble := -D^.VDouble;
        end;
      varCurrency:
        begin
          R^.VType := varCurrency;
          R^.VInt64 := -D^.VInt64;
        end;
      varString, varOleStr, varUString: bruto_var_from_double(Res, -VariantToDouble(D^));
    else
      VarInvalidUnaryOp(D^.VType, opNegate);
    end
  else
    case D^.VType of
      varEmpty: bruto_var_from_int(Res, -1, varShortInt);
      varNull: R^.VType := varNull;
      varSmallInt: bruto_var_from_int(Res, not D^.VSmallInt, varSmallInt);
      varInteger: bruto_var_from_int(Res, not D^.VInteger, varInteger);
      varShortInt: bruto_var_from_int(Res, not D^.VShortInt, varShortInt);
      varByte: bruto_var_from_int(Res, Byte(not D^.VByte), varByte);
      varWord: bruto_var_from_int(Res, Word(not D^.VWord), varWord);
      varLongWord: bruto_var_from_int(Res, LongWord(not D^.VLongWord), varLongWord);
      varInt64: bruto_var_from_int(Res, not D^.VInt64, varInt64);
      varQWord: bruto_var_from_int(Res, not D^.VInt64, varQWord);
      varBoolean: bruto_var_from_bool(Res, Ord(not D^.VBoolean));
      varSingle, varDouble, varDate, varCurrency, varString, varOleStr, varUString:
        begin
          I := VariantToInt64(D^);
          if (I < Low(LongInt)) or (I > High(LongInt)) then
            bruto_var_from_int(Res, not I, varInt64)
          else
            bruto_var_from_int(Res, not I, varInteger);
        end;
    else
      VarInvalidUnaryOp(D^.VType, opNot);
    end;
end;

procedure bruto_var_array_get(Res, A, Indices: Pointer; Count: Int64);
var
  E: PVarData;
  Dims: Int64;
  T: TVarData;
begin
  E := ElementAt(PVarData(A)^, PInt64Array(Indices), Count);
  Dims := DimsOf(PVarData(A)^.VArray);
  if Count > Dims then
  begin
    { The element is itself an array, indexed by the rest. }
    FillChar(T, SizeOf(T), 0);
    CopyData(T, E^);
    try
      bruto_var_array_get(Res, @T, Indices + Dims * 8, Count - Dims);
    finally
      ClearData(T);
    end;
  end
  else
    CopyData(PVarData(Res)^, E^);
end;

procedure bruto_var_array_put(A, Indices: Pointer; Count: Int64; V: Pointer);
var
  D, E: PVarData;
  Dims: Int64;
  T: TVarData;
begin
  D := PVarData(A);
  if (D^.VType and varArray) = 0 then
    VarInvalidArgError;
  UniqueArray(D^);
  E := ElementAt(D^, PInt64Array(Indices), Count);
  Dims := DimsOf(D^.VArray);
  if Count > Dims then
  begin
    bruto_var_array_put(E, Indices + Dims * 8, Count - Dims, V);
    Exit;
  end;
  FillChar(T, SizeOf(T), 0);
  if (D^.VType and varTypeMask) = varVariant then
  begin
    { AnsiStrings are stored as OleStr, as FPC does. }
    if PVarData(V)^.VType = varString then
      CastData(T, PVarData(V)^, varOleStr)
    else
      CopyData(T, PVarData(V)^);
  end
  else
    CastData(T, PVarData(V)^, D^.VType and varTypeMask);
  ClearData(E^);
  Move(T, E^, SizeOf(TVarData));
end;

{ ── Properties ─────────────────────────────────────────────── }

function GetVariantProp(Instance: TObject; PropInfo: PPropInfo): Variant;
begin
  Result := TypInfo.GetVariantProp(Instance, PropInfo);
end;

function GetVariantProp(Instance: TObject; const PropName: string): Variant;
begin
  Result := TypInfo.GetVariantProp(Instance, PropName);
end;

procedure SetVariantProp(Instance: TObject; PropInfo: PPropInfo; const Value: Variant);
begin
  TypInfo.SetVariantProp(Instance, PropInfo, Value);
end;

procedure SetVariantProp(Instance: TObject; const PropName: string; const Value: Variant);
begin
  TypInfo.SetVariantProp(Instance, PropName, Value);
end;

function GetPropValue(Instance: TObject; const PropName: string): Variant;
begin
  Result := GetPropValue(Instance, PropName, True);
end;

function GetPropValue(Instance: TObject; const PropName: string; PreferStrings: Boolean): Variant;
var
  Info: PPropInfo;
begin
  Info := FindPropInfo(Instance, PropName);
  case Info^.PropType^.Kind of
    tkInteger, tkChar, tkWChar: Result := GetOrdProp(Instance, Info);
    tkInt64, tkQWord: Result := GetInt64Prop(Instance, Info);
    tkBool: Result := GetOrdProp(Instance, Info) <> 0;
    tkEnumeration:
      if PreferStrings then
        Result := GetEnumProp(Instance, Info)
      else
        Result := GetOrdProp(Instance, Info);
    tkSet:
      if PreferStrings then
        Result := GetSetProp(Instance, Info, False)
      else
        Result := GetOrdProp(Instance, Info);
    tkFloat: Result := GetFloatProp(Instance, Info);
    tkSString, tkLString, tkAString: Result := GetStrProp(Instance, Info);
    tkUString, tkWString: Result := UnicodeString(GetStrProp(Instance, Info));
    tkVariant: Result := GetVariantProp(Instance, Info);
  else
    raise EPropertyError.Create('Invalid property type: ' + Info^.PropType^.Name);
  end;
end;

procedure SetPropValue(Instance: TObject; const PropName: string; const Value: Variant);
var
  Info: PPropInfo;
begin
  Info := FindPropInfo(Instance, PropName);
  case Info^.PropType^.Kind of
    tkInteger, tkChar, tkWChar, tkBool: SetOrdProp(Instance, Info, Int64(Value));
    tkInt64, tkQWord: SetInt64Prop(Instance, Info, Int64(Value));
    tkEnumeration:
      if VarIsStr(Value) then
        SetEnumProp(Instance, Info, VarToStr(Value))
      else
        SetOrdProp(Instance, Info, Int64(Value));
    tkSet:
      if VarIsStr(Value) then
        SetSetProp(Instance, Info, VarToStr(Value))
      else
        SetOrdProp(Instance, Info, Int64(Value));
    tkFloat: SetFloatProp(Instance, Info, Double(Value));
    tkSString, tkLString, tkAString: SetStrProp(Instance, Info, VarToStr(Value));
    tkUString, tkWString: SetStrProp(Instance, Info, VarToStr(Value));
    tkVariant: SetVariantProp(Instance, Info, Value);
  else
    raise EPropertyError.Create('Invalid property type: ' + Info^.PropType^.Name);
  end;
end;

end.
