{ Bruto Pascal runtime: FPC's TypInfo unit, reading the type information
  the compiler makes (TTypeInfo, TTypeData and TPropInfo, declared in the
  System prelude): enumeration names, sets as text, and the published
  properties of classes, read and written by name. }
unit TypInfo;

{$mode objfpc}{$H+}

interface

uses SysUtils;

const
  ptField = 0;
  ptStatic = 1;
  ptVirtual = 2;
  ptConst = 3;

type
  TTypeKinds = set of TTypeKind;
  PPropList = ^TPropList;
  TPropList = array[0..65535] of PPropInfo;
  EPropertyError = class(Exception);

const
  tkAny = [Low(TTypeKind)..High(TTypeKind)];
  tkMethods = [tkMethod];
  tkProperties = tkAny - tkMethods - [tkUnknown];
  BooleanIdents: array[Boolean] of string = ('False', 'True');

function GetTypeData(TypeInfo: PTypeInfo): PTypeData;

function GetEnumName(TypeInfo: PTypeInfo; Value: Integer): string;
function GetEnumValue(TypeInfo: PTypeInfo; const Name: string): Integer;
function GetEnumNameCount(Enum1: PTypeInfo): LongInt;
function SetToString(TypeInfo: PTypeInfo; Value: LongInt; Brackets: Boolean): string; overload;
function SetToString(PropInfo: PPropInfo; Value: LongInt; Brackets: Boolean): string; overload;
function SetToString(PropInfo: PPropInfo; Value: LongInt): string; overload;
function StringToSet(TypeInfo: PTypeInfo; const Value: string): LongInt; overload;
function StringToSet(PropInfo: PPropInfo; const Value: string): LongInt; overload;

function GetPropInfo(TypeInfo: PTypeInfo; const PropName: string): PPropInfo; overload;
function GetPropInfo(Instance: TObject; const PropName: string): PPropInfo; overload;
function GetPropInfo(AClass: TClass; const PropName: string): PPropInfo; overload;
function FindPropInfo(Instance: TObject; const PropName: string): PPropInfo; overload;
function FindPropInfo(AClass: TClass; const PropName: string): PPropInfo; overload;
procedure GetPropInfos(TypeInfo: PTypeInfo; PropList: PPropList);
function GetPropList(TypeInfo: PTypeInfo; TypeKinds: TTypeKinds; PropList: PPropList;
  Sorted: Boolean = True): LongInt; overload;
function GetPropList(TypeInfo: PTypeInfo; out PropList: PPropList): SizeInt; overload;
function GetPropList(AClass: TClass; out PropList: PPropList): Integer; overload;
function GetPropList(Instance: TObject; out PropList: PPropList): Integer; overload;

function IsPublishedProp(Instance: TObject; const PropName: string): Boolean; overload;
function IsPublishedProp(AClass: TClass; const PropName: string): Boolean; overload;
function IsStoredProp(Instance: TObject; PropInfo: PPropInfo): Boolean; overload;
function IsStoredProp(Instance: TObject; const PropName: string): Boolean; overload;
function PropIsType(Instance: TObject; const PropName: string; TypeKind: TTypeKind): Boolean; overload;
function PropIsType(AClass: TClass; const PropName: string; TypeKind: TTypeKind): Boolean; overload;
function PropType(Instance: TObject; const PropName: string): TTypeKind; overload;
function PropType(AClass: TClass; const PropName: string): TTypeKind; overload;

function GetOrdProp(Instance: TObject; PropInfo: PPropInfo): Int64; overload;
function GetOrdProp(Instance: TObject; const PropName: string): Int64; overload;
procedure SetOrdProp(Instance: TObject; PropInfo: PPropInfo; Value: Int64); overload;
procedure SetOrdProp(Instance: TObject; const PropName: string; Value: Int64); overload;
function GetEnumProp(Instance: TObject; PropInfo: PPropInfo): string; overload;
function GetEnumProp(Instance: TObject; const PropName: string): string; overload;
procedure SetEnumProp(Instance: TObject; PropInfo: PPropInfo; const Value: string); overload;
procedure SetEnumProp(Instance: TObject; const PropName: string; const Value: string); overload;
function GetSetProp(Instance: TObject; PropInfo: PPropInfo; Brackets: Boolean): string; overload;
function GetSetProp(Instance: TObject; const PropName: string): string; overload;
function GetSetProp(Instance: TObject; const PropName: string; Brackets: Boolean): string; overload;
procedure SetSetProp(Instance: TObject; PropInfo: PPropInfo; const Value: string); overload;
procedure SetSetProp(Instance: TObject; const PropName: string; const Value: string); overload;
function GetInt64Prop(Instance: TObject; PropInfo: PPropInfo): Int64; overload;
function GetInt64Prop(Instance: TObject; const PropName: string): Int64; overload;
procedure SetInt64Prop(Instance: TObject; PropInfo: PPropInfo; const Value: Int64); overload;
procedure SetInt64Prop(Instance: TObject; const PropName: string; const Value: Int64); overload;
function GetStrProp(Instance: TObject; PropInfo: PPropInfo): AnsiString; overload;
function GetStrProp(Instance: TObject; const PropName: string): string; overload;
procedure SetStrProp(Instance: TObject; PropInfo: PPropInfo; const Value: AnsiString); overload;
procedure SetStrProp(Instance: TObject; const PropName: string; const Value: AnsiString); overload;
function GetFloatProp(Instance: TObject; PropInfo: PPropInfo): Double; overload;
function GetFloatProp(Instance: TObject; const PropName: string): Double; overload;
procedure SetFloatProp(Instance: TObject; PropInfo: PPropInfo; Value: Double); overload;
procedure SetFloatProp(Instance: TObject; const PropName: string; Value: Double); overload;
function GetObjectProp(Instance: TObject; PropInfo: PPropInfo): TObject; overload;
function GetObjectProp(Instance: TObject; const PropName: string): TObject; overload;
procedure SetObjectProp(Instance: TObject; PropInfo: PPropInfo; Value: TObject); overload;
procedure SetObjectProp(Instance: TObject; const PropName: string; Value: TObject); overload;
function GetObjectPropClass(Instance: TObject; const PropName: string): TClass;
function GetMethodProp(Instance: TObject; PropInfo: PPropInfo): TMethod; overload;
function GetMethodProp(Instance: TObject; const PropName: string): TMethod; overload;
procedure SetMethodProp(Instance: TObject; PropInfo: PPropInfo; const Value: TMethod); overload;
procedure SetMethodProp(Instance: TObject; const PropName: string; const Value: TMethod); overload;

implementation

type
  PBrutoShortStrings = ^TBrutoShortStrings;
  TBrutoShortStrings = array[0..65535] of PShortString;
  PBrutoProps = ^TBrutoProps;
  TBrutoProps = array[0..65535] of TPropInfo;
  PBrutoCode = ^Pointer;

  TGetOrd = function(Self: Pointer): Int64;
  TGetOrdIndex = function(Self: Pointer; Index: LongInt): Int64;
  TSetOrd = procedure(Self: Pointer; Value: Int64);
  TSetOrdIndex = procedure(Self: Pointer; Index: LongInt; Value: Int64);
  TGetStr = function(Self: Pointer): AnsiString;
  TGetStrIndex = function(Self: Pointer; Index: LongInt): AnsiString;
  TSetStr = procedure(Self: Pointer; const Value: AnsiString);
  TSetStrIndex = procedure(Self: Pointer; Index: LongInt; const Value: AnsiString);
  TGetDouble = function(Self: Pointer): Double;
  TGetDoubleIndex = function(Self: Pointer; Index: LongInt): Double;
  TSetDouble = procedure(Self: Pointer; Value: Double);
  TSetDoubleIndex = procedure(Self: Pointer; Index: LongInt; Value: Double);
  TGetSingle = function(Self: Pointer): Single;
  TSetSingle = procedure(Self: Pointer; Value: Single);
  TGetBool = function(Self: Pointer): Boolean;
  TGetBoolIndex = function(Self: Pointer; Index: LongInt): Boolean;

function GetTypeData(TypeInfo: PTypeInfo): PTypeData;
begin
  { After the TTypeInfo, 8-byte aligned. }
  Result := PTypeData(Pointer(TypeInfo) + ((SizeOf(TTypeInfo) + 7) and not 7));
end;

function GetEnumName(TypeInfo: PTypeInfo; Value: Integer): string;
var
  TD: PTypeData;
  P: PShortString;
begin
  Result := '';
  TD := GetTypeData(TypeInfo);
  case TypeInfo^.Kind of
    tkBool:
      if (Value = 0) or (Value = 1) then
        Result := BooleanIdents[Value = 1];
    tkEnumeration:
      if (Value >= TD^.MinValue) and (Value <= TD^.MaxValue) then
      begin
        P := PBrutoShortStrings(TD^.NameList)^[Value - TD^.MinValue];
        if P <> nil then
          Result := P^;
      end;
    tkInteger:
      Result := IntToStr(Value);
  end;
end;

function GetEnumValue(TypeInfo: PTypeInfo; const Name: string): Integer;
var
  TD: PTypeData;
  P: PShortString;
  I: Integer;
begin
  Result := -1;
  if Name = '' then
    Exit;
  TD := GetTypeData(TypeInfo);
  if TypeInfo^.Kind = tkBool then
  begin
    if CompareText(BooleanIdents[False], Name) = 0 then
      Result := 0
    else if CompareText(BooleanIdents[True], Name) = 0 then
      Result := 1;
    Exit;
  end;
  if TypeInfo^.Kind <> tkEnumeration then
    Exit;
  for I := TD^.MinValue to TD^.MaxValue do
  begin
    P := PBrutoShortStrings(TD^.NameList)^[I - TD^.MinValue];
    if (P <> nil) and (CompareText(P^, Name) = 0) then
      Exit(I);
  end;
end;

function GetEnumNameCount(Enum1: PTypeInfo): LongInt;
var
  TD: PTypeData;
  I: Integer;
begin
  Result := 0;
  TD := GetTypeData(Enum1);
  if Enum1^.Kind = tkBool then
    Exit(2);
  for I := TD^.MinValue to TD^.MaxValue do
    if PBrutoShortStrings(TD^.NameList)^[I - TD^.MinValue] <> nil then
      Inc(Result);
end;

function SetToString(TypeInfo: PTypeInfo; Value: LongInt; Brackets: Boolean): string;
var
  Comp: PTypeInfo;
  I: Integer;
begin
  Result := '';
  Comp := GetTypeData(TypeInfo)^.CompType;
  for I := 0 to 31 do
    if (Value and (LongInt(1) shl I)) <> 0 then
    begin
      if Result <> '' then
        Result := Result + ',';
      if Comp <> nil then
        Result := Result + GetEnumName(Comp, I)
      else
        Result := Result + IntToStr(I);
    end;
  if Brackets then
    Result := '[' + Result + ']';
end;

function SetToString(PropInfo: PPropInfo; Value: LongInt; Brackets: Boolean): string;
begin
  Result := SetToString(PropInfo^.PropType, Value, Brackets);
end;

function SetToString(PropInfo: PPropInfo; Value: LongInt): string;
begin
  Result := SetToString(PropInfo^.PropType, Value, False);
end;

function StringToSet(TypeInfo: PTypeInfo; const Value: string): LongInt;
var
  Comp: PTypeInfo;
  S, Item: string;
  P, N: Integer;
begin
  Result := 0;
  Comp := GetTypeData(TypeInfo)^.CompType;
  S := Trim(Value);
  if (S <> '') and (S[1] = '[') then
    Delete(S, 1, 1);
  if (S <> '') and (S[Length(S)] = ']') then
    Delete(S, Length(S), 1);
  while S <> '' do
  begin
    P := Pos(',', S);
    if P = 0 then
    begin
      Item := Trim(S);
      S := '';
    end
    else
    begin
      Item := Trim(Copy(S, 1, P - 1));
      Delete(S, 1, P);
    end;
    if Item = '' then
      Continue;
    if Comp <> nil then
      N := GetEnumValue(Comp, Item)
    else
      N := StrToIntDef(Item, -1);
    if N < 0 then
      raise EPropertyError.CreateFmt('Invalid set element: "%s"', [Item]);
    Result := Result or (LongInt(1) shl N);
  end;
end;

function StringToSet(PropInfo: PPropInfo; const Value: string): LongInt;
begin
  Result := StringToSet(PropInfo^.PropType, Value);
end;

{ ── Properties ── }

function OwnProps(TD: PTypeData): PBrutoProps;
begin
  Result := PBrutoProps(TD^.OwnProps);
end;

function GetPropInfo(TypeInfo: PTypeInfo; const PropName: string): PPropInfo;
var
  TD: PTypeData;
  I: Integer;
begin
  Result := nil;
  while TypeInfo <> nil do
  begin
    TD := GetTypeData(TypeInfo);
    for I := 0 to TD^.OwnPropCount - 1 do
      if CompareText(OwnProps(TD)^[I].Name, PropName) = 0 then
        Exit(@OwnProps(TD)^[I]);
    TypeInfo := TD^.ParentInfo;
  end;
end;

function GetPropInfo(Instance: TObject; const PropName: string): PPropInfo;
begin
  Result := GetPropInfo(PTypeInfo(Instance.ClassInfo), PropName);
end;

function GetPropInfo(AClass: TClass; const PropName: string): PPropInfo;
begin
  Result := GetPropInfo(PTypeInfo(AClass.ClassInfo), PropName);
end;

function FindPropInfo(Instance: TObject; const PropName: string): PPropInfo;
begin
  Result := GetPropInfo(Instance, PropName);
  if Result = nil then
    raise EPropertyError.CreateFmt('Unknown property: "%s"', [PropName]);
end;

function FindPropInfo(AClass: TClass; const PropName: string): PPropInfo;
begin
  Result := GetPropInfo(AClass, PropName);
  if Result = nil then
    raise EPropertyError.CreateFmt('Unknown property: "%s"', [PropName]);
end;

procedure GetPropInfos(TypeInfo: PTypeInfo; PropList: PPropList);
var
  TD: PTypeData;
  I: Integer;
  P: PPropInfo;
begin
  TD := GetTypeData(TypeInfo);
  FillChar(PropList^, TD^.PropCount * SizeOf(Pointer), 0);
  while TypeInfo <> nil do
  begin
    TD := GetTypeData(TypeInfo);
    for I := 0 to TD^.OwnPropCount - 1 do
    begin
      P := @OwnProps(TD)^[I];
      if PropList^[P^.NameIndex] = nil then
        PropList^[P^.NameIndex] := P;
    end;
    TypeInfo := TD^.ParentInfo;
  end;
end;

function GetPropList(TypeInfo: PTypeInfo; TypeKinds: TTypeKinds; PropList: PPropList;
  Sorted: Boolean): LongInt;
var
  All: PPropList;
  Total, I, J: Integer;
  P: PPropInfo;
begin
  Result := 0;
  Total := GetTypeData(TypeInfo)^.PropCount;
  if Total = 0 then
    Exit;
  GetMem(All, Total * SizeOf(Pointer));
  GetPropInfos(TypeInfo, All);
  for I := 0 to Total - 1 do
  begin
    P := All^[I];
    if (P = nil) or not (P^.PropType^.Kind in TypeKinds) then
      Continue;
    if PropList <> nil then
    begin
      J := Result;
      if Sorted then
        while (J > 0) and (CompareText(PropList^[J - 1]^.Name, P^.Name) > 0) do
        begin
          PropList^[J] := PropList^[J - 1];
          Dec(J);
        end;
      PropList^[J] := P;
    end;
    Inc(Result);
  end;
  FreeMem(All);
end;

function GetPropList(TypeInfo: PTypeInfo; out PropList: PPropList): SizeInt;
begin
  Result := GetTypeData(TypeInfo)^.PropCount;
  if Result > 0 then
  begin
    GetMem(PropList, Result * SizeOf(Pointer));
    GetPropInfos(TypeInfo, PropList);
  end
  else
    PropList := nil;
end;

function GetPropList(AClass: TClass; out PropList: PPropList): Integer;
begin
  Result := GetPropList(PTypeInfo(AClass.ClassInfo), PropList);
end;

function GetPropList(Instance: TObject; out PropList: PPropList): Integer;
begin
  Result := GetPropList(PTypeInfo(Instance.ClassInfo), PropList);
end;

function IsPublishedProp(Instance: TObject; const PropName: string): Boolean;
begin
  Result := GetPropInfo(Instance, PropName) <> nil;
end;

function IsPublishedProp(AClass: TClass; const PropName: string): Boolean;
begin
  Result := GetPropInfo(AClass, PropName) <> nil;
end;

function PropType(Instance: TObject; const PropName: string): TTypeKind;
begin
  Result := FindPropInfo(Instance, PropName)^.PropType^.Kind;
end;

function PropType(AClass: TClass; const PropName: string): TTypeKind;
begin
  Result := FindPropInfo(AClass, PropName)^.PropType^.Kind;
end;

function PropIsType(Instance: TObject; const PropName: string; TypeKind: TTypeKind): Boolean;
begin
  Result := PropType(Instance, PropName) = TypeKind;
end;

function PropIsType(AClass: TClass; const PropName: string; TypeKind: TTypeKind): Boolean;
begin
  Result := PropType(AClass, PropName) = TypeKind;
end;

{ The code a ptStatic or ptVirtual accessor calls. }
function AccessorCode(Instance: TObject; Proc: Pointer; Kind: Integer): Pointer;
begin
  if Kind = ptVirtual then
    Result := PBrutoCode(PPointer(Instance)^ + PtrUInt(Proc))^
  else
    Result := Proc;
end;

function HasIndex(PropInfo: PPropInfo): Boolean;
begin
  Result := PropInfo^.Index <> Low(LongInt);
end;

{ An ordinal of the property's type, from its storage. }
function ReadOrd(P: Pointer; PropInfo: PPropInfo): Int64;
var
  T: PTypeInfo;
begin
  T := PropInfo^.PropType;
  case T^.Kind of
    tkInt64, tkQWord, tkClass, tkPointer, tkClassRef, tkProcVar:
      Exit(PInt64(P)^);
  end;
  case GetTypeData(T)^.OrdType of
    otSByte: Result := PShortInt(P)^;
    otUByte: Result := PByte(P)^;
    otSWord: Result := PSmallInt(P)^;
    otUWord: Result := PWord(P)^;
    otSLong: Result := PLongInt(P)^;
    otULong: Result := PLongWord(P)^;
  else
    Result := PInt64(P)^;
  end;
end;

procedure WriteOrd(P: Pointer; PropInfo: PPropInfo; Value: Int64);
var
  T: PTypeInfo;
begin
  T := PropInfo^.PropType;
  case T^.Kind of
    tkInt64, tkQWord, tkClass, tkPointer, tkClassRef, tkProcVar:
      begin
        PInt64(P)^ := Value;
        Exit;
      end;
    tkSet:
      begin
        PLongWord(P)^ := Value;
        Exit;
      end;
  end;
  case GetTypeData(T)^.OrdType of
    otSByte, otUByte: PByte(P)^ := Value;
    otSWord, otUWord: PWord(P)^ := Value;
    otSLong, otULong: PLongWord(P)^ := Value;
  else
    PInt64(P)^ := Value;
  end;
end;

{ A getter's result, cut to the property's type (the call returns more). }
function TrimOrd(Value: Int64; PropInfo: PPropInfo): Int64;
var
  T: PTypeInfo;
begin
  T := PropInfo^.PropType;
  if T^.Kind = tkBool then
    Exit(Value and 1);
  case T^.Kind of
    tkInt64, tkQWord, tkClass, tkPointer, tkClassRef, tkProcVar:
      Exit(Value);
  end;
  case GetTypeData(T)^.OrdType of
    otSByte: Result := ShortInt(Value and $FF);
    otUByte: Result := Value and $FF;
    otSWord: Result := SmallInt(Value and $FFFF);
    otUWord: Result := Value and $FFFF;
    otSLong: Result := LongInt(Value and $FFFFFFFF);
    otULong: Result := Value and $FFFFFFFF;
  else
    Result := Value;
  end;
end;

function GetOrdProp(Instance: TObject; PropInfo: PPropInfo): Int64;
var
  Code: Pointer;
begin
  case PropInfo^.PropProcs and 3 of
    ptField:
      Result := ReadOrd(Pointer(Instance) + PtrUInt(PropInfo^.GetProc), PropInfo);
    ptStatic, ptVirtual:
      begin
        Code := AccessorCode(Instance, PropInfo^.GetProc, PropInfo^.PropProcs and 3);
        if HasIndex(PropInfo) then
          Result := TGetOrdIndex(Code)(Pointer(Instance), PropInfo^.Index)
        else
          Result := TGetOrd(Code)(Pointer(Instance));
        Result := TrimOrd(Result, PropInfo);
      end;
  else
    Result := 0;
  end;
end;

procedure SetOrdProp(Instance: TObject; PropInfo: PPropInfo; Value: Int64);
var
  Code: Pointer;
begin
  case (PropInfo^.PropProcs shr 2) and 3 of
    ptField:
      if PropInfo^.SetProc <> nil then
        WriteOrd(Pointer(Instance) + PtrUInt(PropInfo^.SetProc), PropInfo, Value);
    ptStatic, ptVirtual:
      begin
        Code := AccessorCode(Instance, PropInfo^.SetProc, (PropInfo^.PropProcs shr 2) and 3);
        if HasIndex(PropInfo) then
          TSetOrdIndex(Code)(Pointer(Instance), PropInfo^.Index, Value)
        else
          TSetOrd(Code)(Pointer(Instance), Value);
      end;
  end;
end;

function IsStoredProp(Instance: TObject; PropInfo: PPropInfo): Boolean;
var
  Code: Pointer;
begin
  case (PropInfo^.PropProcs shr 4) and 3 of
    ptConst:
      Result := PropInfo^.StoredProc <> nil;
    ptField:
      Result := PByte(Pointer(Instance) + PtrUInt(PropInfo^.StoredProc))^ <> 0;
  else
    begin
      Code := AccessorCode(Instance, PropInfo^.StoredProc, (PropInfo^.PropProcs shr 4) and 3);
      if HasIndex(PropInfo) then
        Result := TGetBoolIndex(Code)(Pointer(Instance), PropInfo^.Index)
      else
        Result := TGetBool(Code)(Pointer(Instance));
    end;
  end;
end;

function IsStoredProp(Instance: TObject; const PropName: string): Boolean;
begin
  Result := IsStoredProp(Instance, FindPropInfo(Instance, PropName));
end;

function GetOrdProp(Instance: TObject; const PropName: string): Int64;
begin
  Result := GetOrdProp(Instance, FindPropInfo(Instance, PropName));
end;

procedure SetOrdProp(Instance: TObject; const PropName: string; Value: Int64);
begin
  SetOrdProp(Instance, FindPropInfo(Instance, PropName), Value);
end;

function GetEnumProp(Instance: TObject; PropInfo: PPropInfo): string;
begin
  Result := GetEnumName(PropInfo^.PropType, GetOrdProp(Instance, PropInfo));
end;

function GetEnumProp(Instance: TObject; const PropName: string): string;
begin
  Result := GetEnumProp(Instance, FindPropInfo(Instance, PropName));
end;

procedure SetEnumProp(Instance: TObject; PropInfo: PPropInfo; const Value: string);
var
  N: Integer;
begin
  N := GetEnumValue(PropInfo^.PropType, Value);
  if N < 0 then
    raise EPropertyError.CreateFmt('Invalid enumeration value: "%s"', [Value]);
  SetOrdProp(Instance, PropInfo, N);
end;

procedure SetEnumProp(Instance: TObject; const PropName: string; const Value: string);
begin
  SetEnumProp(Instance, FindPropInfo(Instance, PropName), Value);
end;

function GetSetProp(Instance: TObject; PropInfo: PPropInfo; Brackets: Boolean): string;
begin
  Result := SetToString(PropInfo^.PropType, GetOrdProp(Instance, PropInfo), Brackets);
end;

function GetSetProp(Instance: TObject; const PropName: string): string;
begin
  Result := GetSetProp(Instance, FindPropInfo(Instance, PropName), False);
end;

function GetSetProp(Instance: TObject; const PropName: string; Brackets: Boolean): string;
begin
  Result := GetSetProp(Instance, FindPropInfo(Instance, PropName), Brackets);
end;

procedure SetSetProp(Instance: TObject; PropInfo: PPropInfo; const Value: string);
begin
  SetOrdProp(Instance, PropInfo, StringToSet(PropInfo^.PropType, Value));
end;

procedure SetSetProp(Instance: TObject; const PropName: string; const Value: string);
begin
  SetSetProp(Instance, FindPropInfo(Instance, PropName), Value);
end;

function GetInt64Prop(Instance: TObject; PropInfo: PPropInfo): Int64;
begin
  Result := GetOrdProp(Instance, PropInfo);
end;

function GetInt64Prop(Instance: TObject; const PropName: string): Int64;
begin
  Result := GetOrdProp(Instance, PropName);
end;

procedure SetInt64Prop(Instance: TObject; PropInfo: PPropInfo; const Value: Int64);
begin
  SetOrdProp(Instance, PropInfo, Value);
end;

procedure SetInt64Prop(Instance: TObject; const PropName: string; const Value: Int64);
begin
  SetOrdProp(Instance, PropName, Value);
end;

function GetStrProp(Instance: TObject; PropInfo: PPropInfo): AnsiString;
var
  P, Code: Pointer;
begin
  Result := '';
  case PropInfo^.PropProcs and 3 of
    ptField:
      begin
        P := Pointer(Instance) + PtrUInt(PropInfo^.GetProc);
        case PropInfo^.PropType^.Kind of
          tkSString: Result := PShortString(P)^;
          tkUString: Result := PUnicodeString(P)^;
        else
          Result := PAnsiString(P)^;
        end;
      end;
    ptStatic, ptVirtual:
      begin
        Code := AccessorCode(Instance, PropInfo^.GetProc, PropInfo^.PropProcs and 3);
        if HasIndex(PropInfo) then
          Result := TGetStrIndex(Code)(Pointer(Instance), PropInfo^.Index)
        else
          Result := TGetStr(Code)(Pointer(Instance));
      end;
  end;
end;

function GetStrProp(Instance: TObject; const PropName: string): string;
begin
  Result := GetStrProp(Instance, FindPropInfo(Instance, PropName));
end;

procedure SetStrProp(Instance: TObject; PropInfo: PPropInfo; const Value: AnsiString);
var
  P, Code: Pointer;
begin
  case (PropInfo^.PropProcs shr 2) and 3 of
    ptField:
      if PropInfo^.SetProc <> nil then
      begin
        P := Pointer(Instance) + PtrUInt(PropInfo^.SetProc);
        case PropInfo^.PropType^.Kind of
          tkSString: PShortString(P)^ := Value;
          tkUString: PUnicodeString(P)^ := UnicodeString(Value);
        else
          PAnsiString(P)^ := Value;
        end;
      end;
    ptStatic, ptVirtual:
      begin
        Code := AccessorCode(Instance, PropInfo^.SetProc, (PropInfo^.PropProcs shr 2) and 3);
        if HasIndex(PropInfo) then
          TSetStrIndex(Code)(Pointer(Instance), PropInfo^.Index, Value)
        else
          TSetStr(Code)(Pointer(Instance), Value);
      end;
  end;
end;

procedure SetStrProp(Instance: TObject; const PropName: string; const Value: AnsiString);
begin
  SetStrProp(Instance, FindPropInfo(Instance, PropName), Value);
end;

function GetFloatProp(Instance: TObject; PropInfo: PPropInfo): Double;
var
  P, Code: Pointer;
  IsSingle: Boolean;
begin
  Result := 0;
  IsSingle := GetTypeData(PropInfo^.PropType)^.FloatType = ftSingle;
  case PropInfo^.PropProcs and 3 of
    ptField:
      begin
        P := Pointer(Instance) + PtrUInt(PropInfo^.GetProc);
        if IsSingle then
          Result := PSingle(P)^
        else
          Result := PDouble(P)^;
      end;
    ptStatic, ptVirtual:
      begin
        Code := AccessorCode(Instance, PropInfo^.GetProc, PropInfo^.PropProcs and 3);
        if IsSingle then
          Result := TGetSingle(Code)(Pointer(Instance))
        else if HasIndex(PropInfo) then
          Result := TGetDoubleIndex(Code)(Pointer(Instance), PropInfo^.Index)
        else
          Result := TGetDouble(Code)(Pointer(Instance));
      end;
  end;
end;

function GetFloatProp(Instance: TObject; const PropName: string): Double;
begin
  Result := GetFloatProp(Instance, FindPropInfo(Instance, PropName));
end;

procedure SetFloatProp(Instance: TObject; PropInfo: PPropInfo; Value: Double);
var
  P, Code: Pointer;
  IsSingle: Boolean;
begin
  IsSingle := GetTypeData(PropInfo^.PropType)^.FloatType = ftSingle;
  case (PropInfo^.PropProcs shr 2) and 3 of
    ptField:
      if PropInfo^.SetProc <> nil then
      begin
        P := Pointer(Instance) + PtrUInt(PropInfo^.SetProc);
        if IsSingle then
          PSingle(P)^ := Value
        else
          PDouble(P)^ := Value;
      end;
    ptStatic, ptVirtual:
      begin
        Code := AccessorCode(Instance, PropInfo^.SetProc, (PropInfo^.PropProcs shr 2) and 3);
        if IsSingle then
          TSetSingle(Code)(Pointer(Instance), Value)
        else if HasIndex(PropInfo) then
          TSetDoubleIndex(Code)(Pointer(Instance), PropInfo^.Index, Value)
        else
          TSetDouble(Code)(Pointer(Instance), Value);
      end;
  end;
end;

procedure SetFloatProp(Instance: TObject; const PropName: string; Value: Double);
begin
  SetFloatProp(Instance, FindPropInfo(Instance, PropName), Value);
end;

function GetObjectProp(Instance: TObject; PropInfo: PPropInfo): TObject;
begin
  Result := TObject(Pointer(PtrUInt(GetOrdProp(Instance, PropInfo))));
end;

function GetObjectProp(Instance: TObject; const PropName: string): TObject;
begin
  Result := GetObjectProp(Instance, FindPropInfo(Instance, PropName));
end;

procedure SetObjectProp(Instance: TObject; PropInfo: PPropInfo; Value: TObject);
begin
  SetOrdProp(Instance, PropInfo, Int64(PtrUInt(Pointer(Value))));
end;

procedure SetObjectProp(Instance: TObject; const PropName: string; Value: TObject);
begin
  SetObjectProp(Instance, FindPropInfo(Instance, PropName), Value);
end;

function GetObjectPropClass(Instance: TObject; const PropName: string): TClass;
begin
  Result := TClass(GetTypeData(FindPropInfo(Instance, PropName)^.PropType)^.ClassType);
end;

function GetMethodProp(Instance: TObject; PropInfo: PPropInfo): TMethod;
var
  P: Pointer;
begin
  Result.Code := nil;
  Result.Data := nil;
  if (PropInfo^.PropProcs and 3) = ptField then
  begin
    P := Pointer(Instance) + PtrUInt(PropInfo^.GetProc);
    Result := PMethod(P)^;
  end;
end;

function GetMethodProp(Instance: TObject; const PropName: string): TMethod;
begin
  Result := GetMethodProp(Instance, FindPropInfo(Instance, PropName));
end;

procedure SetMethodProp(Instance: TObject; PropInfo: PPropInfo; const Value: TMethod);
var
  P: Pointer;
begin
  if ((PropInfo^.PropProcs shr 2) and 3) = ptField then
  begin
    P := Pointer(Instance) + PtrUInt(PropInfo^.SetProc);
    PMethod(P)^ := Value;
  end;
end;

procedure SetMethodProp(Instance: TObject; const PropName: string; const Value: TMethod);
begin
  SetMethodProp(Instance, FindPropInfo(Instance, PropName), Value);
end;

end.
