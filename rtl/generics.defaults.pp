{ Bruto Pascal runtime: the comparers of FPC's Generics.Defaults.
  IComparer<T> and IEqualityComparer<T>, TComparer<T> and
  TEqualityComparer<T> with their Default and Construct class
  functions (from a method or a plain function), as in FPC 3.2.2.

  The default comparers compare numbers, characters, Booleans,
  enumerations, strings and Variants by value, pointers, classes and
  interfaces by address, and other types (records, static arrays, sets)
  by their bytes, as FPC's binary comparer does; the compiler's
  BrutoCompare, BrutoEquals and BrutoHash built-ins do the work. Hash
  codes are Bruto Pascal's own, not FPC's. }
unit Generics.Defaults;

{$mode objfpc}{$H+}

interface

uses SysUtils;

type
  generic IComparer<T> = interface
    function Compare(constref Left, Right: T): Integer; overload;
  end;

  generic TOnComparison<T> = function(constref Left, Right: T): Integer of object;
  generic TComparisonFunc<T> = function(constref Left, Right: T): Integer;

  generic TComparer<T> = class(TInterfacedObject, specialize IComparer<T>)
  public
    class function Default: specialize IComparer<T>; static;
    function Compare(constref ALeft, ARight: T): Integer; virtual; abstract; overload;
    class function Construct(const AComparison: specialize TOnComparison<T>): specialize IComparer<T>; overload;
    class function Construct(const AComparison: specialize TComparisonFunc<T>): specialize IComparer<T>; overload;
  end;

  { TComparer<T>.Default's comparer. }
  generic TBrutoDefaultComparer<T> = class(specialize TComparer<T>)
  public
    function Compare(constref ALeft, ARight: T): Integer; override;
  end;

  generic TDelegatedComparerEvents<T> = class(specialize TComparer<T>)
  private
    FComparison: specialize TOnComparison<T>;
  public
    function Compare(constref ALeft, ARight: T): Integer; override;
    constructor Create(AComparison: specialize TOnComparison<T>);
  end;

  generic TDelegatedComparerFunc<T> = class(specialize TComparer<T>)
  private
    FComparison: specialize TComparisonFunc<T>;
  public
    function Compare(constref ALeft, ARight: T): Integer; override;
    constructor Create(AComparison: specialize TComparisonFunc<T>);
  end;

  generic IEqualityComparer<T> = interface
    function Equals(constref ALeft, ARight: T): Boolean;
    function GetHashCode(constref AValue: T): UInt32;
  end;

  generic TOnEqualityComparison<T> = function(constref ALeft, ARight: T): Boolean of object;
  generic TEqualityComparisonFunc<T> = function(constref ALeft, ARight: T): Boolean;
  generic TOnHasher<T> = function(constref AValue: T): UInt32 of object;
  generic THasherFunc<T> = function(constref AValue: T): UInt32;

  generic TEqualityComparer<T> = class(TInterfacedObject, specialize IEqualityComparer<T>)
  public
    class function Default: specialize IEqualityComparer<T>; static;
    class function Construct(const AEqualityComparison: specialize TOnEqualityComparison<T>;
      const AHasher: specialize TOnHasher<T>): specialize IEqualityComparer<T>; overload;
    class function Construct(const AEqualityComparison: specialize TEqualityComparisonFunc<T>;
      const AHasher: specialize THasherFunc<T>): specialize IEqualityComparer<T>; overload;
    function Equals(constref ALeft, ARight: T): Boolean; virtual; overload; abstract;
    function GetHashCode(constref AValue: T): UInt32; virtual; overload; abstract;
  end;

  { TEqualityComparer<T>.Default's comparer. }
  generic TBrutoDefaultEqualityComparer<T> = class(specialize TEqualityComparer<T>)
  public
    function Equals(constref ALeft, ARight: T): Boolean; override;
    function GetHashCode(constref AValue: T): UInt32; override;
  end;

  generic TDelegatedEqualityComparerEvents<T> = class(specialize TEqualityComparer<T>)
  private
    FEqualityComparison: specialize TOnEqualityComparison<T>;
    FHasher: specialize TOnHasher<T>;
  public
    function Equals(constref ALeft, ARight: T): Boolean; override;
    function GetHashCode(constref AValue: T): UInt32; override;
    constructor Create(const AEqualityComparison: specialize TOnEqualityComparison<T>;
      const AHasher: specialize TOnHasher<T>);
  end;

  generic TDelegatedEqualityComparerFunc<T> = class(specialize TEqualityComparer<T>)
  private
    FEqualityComparison: specialize TEqualityComparisonFunc<T>;
    FHasher: specialize THasherFunc<T>;
  public
    function Equals(constref ALeft, ARight: T): Boolean; override;
    function GetHashCode(constref AValue: T): UInt32; override;
    constructor Create(const AEqualityComparison: specialize TEqualityComparisonFunc<T>;
      const AHasher: specialize THasherFunc<T>);
  end;

{ The hashes BrutoHash gives: an integer's, and FNV-1a over N bytes or
  N 16-bit code units. }
function bruto_hash_int(V: Int64): QWord;
function bruto_hash_bytes(P: PByte; N: Int64): QWord;
function bruto_hash_words(P: PWord; N: Int64): QWord;

implementation

function bruto_hash_int(V: Int64): QWord;
begin
  Result := QWord(V) * QWord($9E3779B97F4A7C15);
  Result := Result xor (Result shr 29);
end;

function bruto_hash_bytes(P: PByte; N: Int64): QWord;
var
  I: Int64;
begin
  Result := QWord($CBF29CE484222325);
  for I := 0 to N - 1 do
    Result := (Result xor P[I]) * QWord($100000001B3);
end;

function bruto_hash_words(P: PWord; N: Int64): QWord;
var
  I: Int64;
begin
  Result := QWord($CBF29CE484222325);
  for I := 0 to N - 1 do
    Result := (Result xor P[I]) * QWord($100000001B3);
end;

{ TComparer }

class function TComparer.Default: specialize IComparer<T>;
begin
  Result := specialize TBrutoDefaultComparer<T>.Create;
end;

class function TComparer.Construct(const AComparison: specialize TOnComparison<T>): specialize IComparer<T>;
begin
  Result := specialize TDelegatedComparerEvents<T>.Create(AComparison);
end;

class function TComparer.Construct(const AComparison: specialize TComparisonFunc<T>): specialize IComparer<T>;
begin
  Result := specialize TDelegatedComparerFunc<T>.Create(AComparison);
end;

function TBrutoDefaultComparer.Compare(constref ALeft, ARight: T): Integer;
begin
  Result := BrutoCompare(ALeft, ARight);
end;

function TDelegatedComparerEvents.Compare(constref ALeft, ARight: T): Integer;
begin
  Result := FComparison(ALeft, ARight);
end;

constructor TDelegatedComparerEvents.Create(AComparison: specialize TOnComparison<T>);
begin
  FComparison := AComparison;
end;

function TDelegatedComparerFunc.Compare(constref ALeft, ARight: T): Integer;
begin
  Result := FComparison(ALeft, ARight);
end;

constructor TDelegatedComparerFunc.Create(AComparison: specialize TComparisonFunc<T>);
begin
  FComparison := AComparison;
end;

{ TEqualityComparer }

class function TEqualityComparer.Default: specialize IEqualityComparer<T>;
begin
  Result := specialize TBrutoDefaultEqualityComparer<T>.Create;
end;

class function TEqualityComparer.Construct(const AEqualityComparison: specialize TOnEqualityComparison<T>;
  const AHasher: specialize TOnHasher<T>): specialize IEqualityComparer<T>;
begin
  Result := specialize TDelegatedEqualityComparerEvents<T>.Create(AEqualityComparison, AHasher);
end;

class function TEqualityComparer.Construct(const AEqualityComparison: specialize TEqualityComparisonFunc<T>;
  const AHasher: specialize THasherFunc<T>): specialize IEqualityComparer<T>;
begin
  Result := specialize TDelegatedEqualityComparerFunc<T>.Create(AEqualityComparison, AHasher);
end;

function TBrutoDefaultEqualityComparer.Equals(constref ALeft, ARight: T): Boolean;
begin
  Result := BrutoEquals(ALeft, ARight);
end;

function TBrutoDefaultEqualityComparer.GetHashCode(constref AValue: T): UInt32;
begin
  Result := UInt32(BrutoHash(AValue));
end;

function TDelegatedEqualityComparerEvents.Equals(constref ALeft, ARight: T): Boolean;
begin
  Result := FEqualityComparison(ALeft, ARight);
end;

function TDelegatedEqualityComparerEvents.GetHashCode(constref AValue: T): UInt32;
begin
  Result := FHasher(AValue);
end;

constructor TDelegatedEqualityComparerEvents.Create(const AEqualityComparison: specialize TOnEqualityComparison<T>;
  const AHasher: specialize TOnHasher<T>);
begin
  FEqualityComparison := AEqualityComparison;
  FHasher := AHasher;
end;

function TDelegatedEqualityComparerFunc.Equals(constref ALeft, ARight: T): Boolean;
begin
  Result := FEqualityComparison(ALeft, ARight);
end;

function TDelegatedEqualityComparerFunc.GetHashCode(constref AValue: T): UInt32;
begin
  Result := FHasher(AValue);
end;

constructor TDelegatedEqualityComparerFunc.Create(const AEqualityComparison: specialize TEqualityComparisonFunc<T>;
  const AHasher: specialize THasherFunc<T>);
begin
  FEqualityComparison := AEqualityComparison;
  FHasher := AHasher;
end;

end.
